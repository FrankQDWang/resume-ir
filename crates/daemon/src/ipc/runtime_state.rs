use std::sync::{mpsc, Arc, RwLock};
use std::thread;

use meta_store::{ReadMetaStore, SearchProjectionServiceState};

use super::capability::{
    CapabilityMatrix, CoreHealth, CoreReason, CoreState, OptionalRuntimeMatrix,
};
use super::{diagnostics, routes, DaemonFatalError};

#[derive(Clone)]
pub(crate) struct ControlPlaneSnapshot {
    pub(crate) core: CoreHealth,
    pub(crate) runtimes: OptionalRuntimeMatrix,
    pub(crate) capabilities: CapabilityMatrix,
    status: serde_json::Value,
    diagnostics: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct ControlPlaneState {
    inner: Arc<RwLock<ControlPlaneSnapshot>>,
}

/// The single typed writer for one daemon generation's cached control-plane
/// projection. Updates cross a capacity-one channel and are acknowledged only
/// after the read-only snapshot has been replaced.
pub(crate) struct ControlPlanePublisher {
    sender: mpsc::SyncSender<SnapshotUpdate>,
    current: ControlPlaneSnapshot,
    prepared_serving: Option<ControlPlaneSnapshot>,
    stage: RuntimeOwnerStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeOwnerStage {
    Initializing,
    Serving,
    Blocked,
}

struct SnapshotUpdate {
    snapshot: ControlPlaneSnapshot,
    applied: mpsc::SyncSender<()>,
}

impl ControlPlaneState {
    pub(crate) fn initializing() -> (Self, ControlPlanePublisher) {
        let core = CoreHealth::initializing();
        let runtimes = OptionalRuntimeMatrix::initializing();
        let initial = snapshot_without_store(core, runtimes);
        let inner = Arc::new(RwLock::new(initial.clone()));
        let worker_inner = Arc::clone(&inner);
        let (sender, receiver) = mpsc::sync_channel::<SnapshotUpdate>(1);
        thread::spawn(move || {
            while let Ok(update) = receiver.recv() {
                *worker_inner
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = update.snapshot;
                let _ = update.applied.send(());
            }
        });
        (
            Self { inner },
            ControlPlanePublisher {
                sender,
                current: initial,
                prepared_serving: None,
                stage: RuntimeOwnerStage::Initializing,
            },
        )
    }

    pub(crate) fn snapshot(&self) -> ControlPlaneSnapshot {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn status_body(&self) -> String {
        let mut body = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
            .clone();
        replace_ipc_metrics(&mut body, None);
        body.to_string()
    }

    pub(crate) fn diagnostics_body(&self) -> String {
        let mut body = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .diagnostics
            .clone();
        replace_ipc_metrics(&mut body, Some("metrics"));
        body.to_string()
    }
}

impl ControlPlanePublisher {
    pub(crate) fn set_runtimes(
        &mut self,
        runtimes: OptionalRuntimeMatrix,
    ) -> Result<(), DaemonFatalError> {
        debug_assert_eq!(self.stage, RuntimeOwnerStage::Initializing);
        self.publish(snapshot_without_store(self.current.core, runtimes))
    }

    pub(crate) fn mark_blocked(&mut self, reason: CoreReason) -> Result<(), DaemonFatalError> {
        self.mark_blocked_with_runtimes(reason, self.current.runtimes)
    }

    pub(crate) fn mark_blocked_with_runtimes(
        &mut self,
        reason: CoreReason,
        runtimes: OptionalRuntimeMatrix,
    ) -> Result<(), DaemonFatalError> {
        self.prepared_serving = None;
        self.stage = RuntimeOwnerStage::Blocked;
        self.publish(snapshot_without_store(
            CoreHealth::blocked(reason),
            runtimes,
        ))
    }

    /// Builds the first store-backed serving projection without making it
    /// visible. The listener's full route owner publishes it only after the
    /// initializing accept loop has stopped and the query service is live.
    pub(crate) fn prepare_from_store(&mut self, store: &ReadMetaStore) {
        debug_assert_eq!(self.stage, RuntimeOwnerStage::Initializing);
        self.prepared_serving = Some(snapshot_from_store(store, self.current.runtimes));
    }

    pub(crate) fn publish_prepared_serving(&mut self) -> Result<(), DaemonFatalError> {
        let snapshot = self
            .prepared_serving
            .take()
            .ok_or(DaemonFatalError::ControlPlaneFailure)?;
        self.stage = owner_stage(snapshot.core);
        self.publish(snapshot)
    }

    pub(crate) fn refresh_from_store_with_runtimes(
        &mut self,
        store: &ReadMetaStore,
        runtimes: OptionalRuntimeMatrix,
    ) -> Result<(), DaemonFatalError> {
        let snapshot = snapshot_from_store(store, runtimes);
        self.stage = owner_stage(snapshot.core);
        self.publish(snapshot)
    }

    fn publish(&mut self, snapshot: ControlPlaneSnapshot) -> Result<(), DaemonFatalError> {
        let (applied, receipt) = mpsc::sync_channel(0);
        self.sender
            .send(SnapshotUpdate {
                snapshot: snapshot.clone(),
                applied,
            })
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
        receipt
            .recv()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
        self.current = snapshot;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn publish_without_store_for_test(
        &mut self,
        core: CoreHealth,
        runtimes: OptionalRuntimeMatrix,
    ) -> Result<(), DaemonFatalError> {
        self.stage = if core.state == CoreState::Ready {
            RuntimeOwnerStage::Serving
        } else {
            RuntimeOwnerStage::Initializing
        };
        self.publish(snapshot_without_store(core, runtimes))
    }
}

fn owner_stage(core: CoreHealth) -> RuntimeOwnerStage {
    match core.state {
        CoreState::Ready => RuntimeOwnerStage::Serving,
        CoreState::Initializing | CoreState::Repairing => RuntimeOwnerStage::Initializing,
        CoreState::Degraded | CoreState::Blocked => RuntimeOwnerStage::Blocked,
    }
}

fn snapshot_from_store(
    store: &ReadMetaStore,
    runtimes: OptionalRuntimeMatrix,
) -> ControlPlaneSnapshot {
    snapshot_from_fallible_source(
        runtimes,
        || core_health_from_store(store).map_err(|_| ()),
        |core, capabilities| {
            routes::status::render_from_store(store, core, runtimes, capabilities).map_err(|_| ())
        },
        |core, capabilities| {
            diagnostics::render_from_store(store, core, runtimes, capabilities).map_err(|_| ())
        },
    )
}

fn snapshot_from_fallible_source(
    runtimes: OptionalRuntimeMatrix,
    read_core: impl FnOnce() -> Result<CoreHealth, ()>,
    render_status: impl FnOnce(CoreHealth, CapabilityMatrix) -> Result<serde_json::Value, ()>,
    render_diagnostics: impl FnOnce(CoreHealth, CapabilityMatrix) -> Result<serde_json::Value, ()>,
) -> ControlPlaneSnapshot {
    let loaded = (|| {
        let core = read_core()?;
        let capabilities = CapabilityMatrix::derive(core, runtimes);
        let status = render_status(core, capabilities)?;
        let diagnostics = render_diagnostics(core, capabilities)?;
        Ok(ControlPlaneSnapshot {
            core,
            runtimes,
            capabilities,
            status,
            diagnostics,
        })
    })();
    loaded.unwrap_or_else(|()| {
        snapshot_without_store(
            CoreHealth {
                state: CoreState::Degraded,
                reason: Some(CoreReason::MetadataUnavailable),
            },
            runtimes,
        )
    })
}

fn snapshot_without_store(
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
) -> ControlPlaneSnapshot {
    let capabilities = CapabilityMatrix::derive(core, runtimes);
    ControlPlaneSnapshot {
        core,
        runtimes,
        capabilities,
        status: routes::status::render_without_store(core, runtimes, capabilities),
        diagnostics: diagnostics::render_without_store(core, runtimes, capabilities),
    }
}

fn replace_ipc_metrics(body: &mut serde_json::Value, parent: Option<&str>) {
    let metrics = super::process_metrics().snapshot().to_json();
    let target = match parent {
        Some(parent) => body
            .get_mut(parent)
            .and_then(serde_json::Value::as_object_mut)
            .expect("control-plane metrics node is an object"),
        None => body
            .as_object_mut()
            .expect("control-plane status is an object"),
    };
    target.insert("ipc".to_string(), metrics);
}

fn core_health_from_store(store: &ReadMetaStore) -> meta_store::Result<CoreHealth> {
    let projection = store.search_projection_state()?;
    let reason = projection.repair_reason.map(|reason| match reason {
        meta_store::SearchRepairReason::MigrationRebuild => CoreReason::MigrationRebuild,
        meta_store::SearchRepairReason::ArtifactUnavailable => CoreReason::ArtifactUnavailable,
        meta_store::SearchRepairReason::SourceUnavailable => CoreReason::SourceUnavailable,
        meta_store::SearchRepairReason::RuntimeInvariant => CoreReason::RuntimeInvariant,
    });
    Ok(match projection.service_state {
        SearchProjectionServiceState::Ready => CoreHealth {
            state: CoreState::Ready,
            reason: None,
        },
        SearchProjectionServiceState::Repairing => CoreHealth {
            state: CoreState::Repairing,
            reason,
        },
        SearchProjectionServiceState::RepairBlocked => CoreHealth {
            state: CoreState::Blocked,
            reason: reason.or(Some(CoreReason::RuntimeInvariant)),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{snapshot_from_fallible_source, ControlPlaneState};
    use crate::ipc::{
        CapabilityState, CoreHealth, CoreReason, CoreState, OptionalRuntimeHealth,
        OptionalRuntimeMatrix, OptionalRuntimeReason, OptionalRuntimeState,
    };

    #[test]
    fn bounded_publisher_acknowledges_initial_runtime_projection() {
        let (state, mut publisher) = ControlPlaneState::initializing();
        publisher
            .set_runtimes(OptionalRuntimeMatrix {
                embedding: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Missing),
                ocr: OptionalRuntimeHealth::available(),
                classifier: OptionalRuntimeHealth::available(),
            })
            .unwrap();
        let snapshot = state.snapshot();
        let status = snapshot.status;

        assert_eq!(status["schema_version"], "daemon.status.v3");
        assert_eq!(snapshot.core.state, CoreState::Initializing);
        assert_eq!(
            status["optional_runtimes"]["embedding"]["reason"],
            "missing"
        );
        assert_eq!(
            status["capabilities"]["keyword_search"]["state"],
            "initializing"
        );
    }

    #[test]
    fn ready_embedding_runtime_failure_reprojects_capabilities_in_place() {
        let (state, mut publisher) = ControlPlaneState::initializing();
        let ready_core = crate::ipc::CoreHealth {
            state: CoreState::Ready,
            reason: None,
        };
        let ready_runtimes = OptionalRuntimeMatrix {
            embedding: OptionalRuntimeHealth::available(),
            ocr: OptionalRuntimeHealth::available(),
            classifier: OptionalRuntimeHealth::available(),
        };
        publisher
            .publish_without_store_for_test(ready_core, ready_runtimes)
            .unwrap();
        assert_eq!(
            state.snapshot().capabilities.semantic_search.state,
            CapabilityState::Available
        );

        publisher
            .publish_without_store_for_test(
                ready_core,
                OptionalRuntimeMatrix {
                    embedding: OptionalRuntimeHealth::unavailable(
                        OptionalRuntimeReason::StartFailed,
                    ),
                    ..ready_runtimes
                },
            )
            .unwrap();
        let failed = state.snapshot();
        assert_eq!(failed.core.state, CoreState::Ready);
        assert_eq!(
            failed.runtimes.embedding.state,
            OptionalRuntimeState::Unavailable
        );
        assert_eq!(
            failed.capabilities.keyword_search.state,
            CapabilityState::Available
        );
        assert_eq!(failed.capabilities.detail.state, CapabilityState::Available);
        assert_eq!(
            failed.capabilities.semantic_search.state,
            CapabilityState::Unavailable
        );
        assert_eq!(
            failed.capabilities.hybrid_search.state,
            CapabilityState::Degraded
        );
        assert_eq!(
            failed.capabilities.index_publication.state,
            CapabilityState::Unavailable
        );
        let status = failed.status;
        assert_eq!(
            status["optional_runtimes"]["embedding"]["reason"],
            "start_failed"
        );
    }

    #[test]
    fn later_metadata_failure_replaces_authorization_and_both_renderings_atomically() {
        let calls = Cell::new(0);
        let runtimes = OptionalRuntimeMatrix {
            embedding: OptionalRuntimeHealth::available(),
            ocr: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Invalid),
            classifier: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Missing),
        };
        let snapshot = snapshot_from_fallible_source(
            runtimes,
            || {
                calls.set(1);
                Ok(CoreHealth {
                    state: CoreState::Ready,
                    reason: None,
                })
            },
            |_, _| {
                calls.set(2);
                Ok(serde_json::json!({"core": {"state": "ready"}}))
            },
            |_, _| {
                calls.set(3);
                Err(())
            },
        );

        assert_eq!(calls.get(), 3);
        assert_eq!(snapshot.core.state, CoreState::Degraded);
        assert_eq!(snapshot.core.reason, Some(CoreReason::MetadataUnavailable));
        assert_eq!(
            snapshot.capabilities.keyword_search.state,
            CapabilityState::Blocked
        );
        assert_eq!(snapshot.runtimes, runtimes);
        for body in [&snapshot.status, &snapshot.diagnostics] {
            assert_eq!(body["core"]["state"], "degraded");
            assert_eq!(body["core"]["reason"], "metadata_unavailable");
            assert_eq!(body["optional_runtimes"]["embedding"]["state"], "available");
            assert_eq!(body["optional_runtimes"]["ocr"]["reason"], "invalid");
            assert_eq!(body["optional_runtimes"]["classifier"]["reason"], "missing");
        }
    }
}
