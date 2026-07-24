use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use embedder::{ResidentEmbeddingClient, ResidentEmbeddingStatus};
use meta_store::ReadMetaStore;

use super::{
    ControlPlanePublisher, CoreReason, DaemonFatalError, OptionalRuntimeHealth,
    OptionalRuntimeMatrix, OptionalRuntimeReason, RuntimeHealthReceiver, RuntimeHealthUpdate,
};

const SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

/// Owns all post-bootstrap metadata reads used to update the cached control
/// plane. Request threads only read the in-memory snapshot.
pub(crate) struct StatusUpdater {
    stop: Arc<AtomicBool>,
    join: JoinHandle<Result<(), DaemonFatalError>>,
}

impl StatusUpdater {
    pub(crate) fn start(
        data_dir: &Path,
        mut publisher: ControlPlanePublisher,
        embedding: Option<ResidentEmbeddingClient>,
        mut runtimes: OptionalRuntimeMatrix,
        runtime_health: Option<RuntimeHealthReceiver>,
    ) -> Self {
        let data_dir = data_dir.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            let mut store = None;
            while !worker_stop.load(Ordering::Acquire) {
                merge_runtime_health_updates(&mut runtimes, runtime_health.as_ref());
                if let Some(embedding) = embedding.as_ref() {
                    runtimes.embedding = embedding_health(embedding.status());
                }
                if store.is_none() {
                    match ReadMetaStore::open_data_dir(&data_dir) {
                        Ok(opened) => store = Some(opened),
                        Err(_) => {
                            publisher.mark_blocked_with_runtimes(
                                CoreReason::MetadataUnavailable,
                                runtimes,
                            )?;
                            thread::sleep(SNAPSHOT_REFRESH_INTERVAL);
                            continue;
                        }
                    }
                }
                publisher.refresh_from_store_with_runtimes(
                    store.as_ref().expect("opened metadata reader"),
                    runtimes,
                )?;
                thread::sleep(SNAPSHOT_REFRESH_INTERVAL);
            }
            Ok(())
        });
        Self { stop, join }
    }

    pub(crate) fn check_health(&self) -> Result<(), DaemonFatalError> {
        if self.join.is_finished() && !self.stop.load(Ordering::Acquire) {
            return Err(DaemonFatalError::ControlPlaneFailure);
        }
        Ok(())
    }

    pub(crate) fn shutdown(self) -> Result<(), DaemonFatalError> {
        self.stop.store(true, Ordering::Release);
        self.join
            .join()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?
    }
}

fn merge_runtime_health_updates(
    runtimes: &mut OptionalRuntimeMatrix,
    receiver: Option<&RuntimeHealthReceiver>,
) {
    let Some(receiver) = receiver else {
        return;
    };
    while let Ok(Some(update)) = receiver.try_recv() {
        match update {
            RuntimeHealthUpdate::Ocr(health) => runtimes.ocr = health,
        }
    }
}

fn embedding_health(status: ResidentEmbeddingStatus) -> OptionalRuntimeHealth {
    match status {
        ResidentEmbeddingStatus::Ready => OptionalRuntimeHealth::available(),
        ResidentEmbeddingStatus::Starting
        | ResidentEmbeddingStatus::Restarting
        | ResidentEmbeddingStatus::Unavailable
        | ResidentEmbeddingStatus::Shutdown => {
            OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::StartFailed)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use embedder::ResidentEmbeddingStatus;

    use super::{embedding_health, merge_runtime_health_updates, StatusUpdater};
    use crate::ipc::{
        runtime_health_channel, ControlPlaneState, CoreState, OptionalRuntimeHealth,
        OptionalRuntimeMatrix, OptionalRuntimeReason, OptionalRuntimeState,
    };

    #[test]
    fn non_ready_embedding_states_fail_closed() {
        assert_eq!(
            embedding_health(ResidentEmbeddingStatus::Ready).state,
            OptionalRuntimeState::Available
        );
        for status in [
            ResidentEmbeddingStatus::Starting,
            ResidentEmbeddingStatus::Restarting,
            ResidentEmbeddingStatus::Unavailable,
            ResidentEmbeddingStatus::Shutdown,
        ] {
            let health = embedding_health(status);
            assert_eq!(health.state, OptionalRuntimeState::Unavailable);
            assert_eq!(health.reason, Some(OptionalRuntimeReason::StartFailed));
        }
    }

    #[test]
    fn updater_merges_typed_runtime_degradation_without_changing_other_runtimes() {
        let (reporter, receiver) = runtime_health_channel();
        let available = OptionalRuntimeHealth::available();
        let mut runtimes = OptionalRuntimeMatrix {
            embedding: available,
            ocr: available,
            classifier: available,
        };
        reporter
            .ocr_unavailable(OptionalRuntimeReason::Invalid)
            .unwrap();

        merge_runtime_health_updates(&mut runtimes, Some(&receiver));

        assert_eq!(runtimes.embedding, available);
        assert_eq!(runtimes.classifier, available);
        assert_eq!(runtimes.ocr.state, OptionalRuntimeState::Unavailable);
        assert_eq!(runtimes.ocr.reason, Some(OptionalRuntimeReason::Invalid));
    }

    #[test]
    fn persistent_metadata_open_failure_publishes_blocked_and_stays_healthy() {
        let directory = tempfile::tempdir().unwrap();
        let available = OptionalRuntimeHealth::available();
        let runtimes = OptionalRuntimeMatrix {
            embedding: available,
            ocr: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Invalid),
            classifier: available,
        };
        let (state, mut publisher) = ControlPlaneState::initializing();
        publisher.set_runtimes(runtimes).unwrap();
        let updater = StatusUpdater::start(directory.path(), publisher, None, runtimes, None);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = state.snapshot();
            if snapshot.core.state == CoreState::Blocked {
                assert_eq!(snapshot.runtimes, runtimes);
                assert_eq!(
                    snapshot.capabilities.keyword_search.state.label(),
                    "blocked"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "blocked snapshot was not published"
            );
            thread::sleep(Duration::from_millis(10));
        }

        thread::sleep(Duration::from_millis(300));
        assert_eq!(updater.check_health(), Ok(()));
        assert_eq!(state.snapshot().core.state, CoreState::Blocked);
        updater.shutdown().unwrap();
    }
}
