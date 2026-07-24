use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore, StoreStatusSummary};

use super::search_service::{ArtifactFaultReporter, SearchService};
use super::status_updater::StatusUpdater;
use super::{
    connection, ControlPlanePublisher, ControlPlaneState, DaemonFatalError, DaemonGenerationOwner,
    GenerationError, RuntimeEvent, RuntimeHealthReceiver,
};
use crate::search_runtime_config::SearchRuntimeConfig;

mod connection_lifecycle;

use connection_lifecycle::{
    handle_business_with_watchdog, run_control_loop, ControlLoopConfig, LISTENER_POLL_INTERVAL,
};

enum ServerStop {
    RequestLimitReached,
    ParentShutdown,
    Fatal(DaemonFatalError),
}

pub(crate) struct Context<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) store: &'a ReadMetaStore,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) max_requests: Option<usize>,
    pub(crate) search_runtime_config: SearchRuntimeConfig,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) shutdown: Option<&'a Arc<AtomicBool>>,
    pub(crate) worker_result_receiver: Option<&'a Receiver<Result<(), DaemonFatalError>>>,
    pub(crate) artifact_fault_reporter: Option<ArtifactFaultReporter>,
    pub(crate) control_state: ControlPlaneState,
    pub(crate) control_publisher: Option<ControlPlanePublisher>,
    pub(crate) runtime_health_receiver: Option<RuntimeHealthReceiver>,
}

/// A listener whose generation credentials and endpoint manifest have already
/// been published. Background data services must not start until this control
/// plane capability exists.
pub(crate) struct BoundServer {
    listener: Option<TcpListener>,
    daemon_owner: Option<DaemonGenerationOwner>,
    handoff_complete: bool,
}

impl BoundServer {
    pub(crate) fn bind(
        addr: SocketAddr,
        daemon_owner: DaemonGenerationOwner,
    ) -> Result<Self, DaemonFatalError> {
        let listener = bind(addr, &daemon_owner)?;
        Ok(Self {
            listener: Some(listener),
            daemon_owner: Some(daemon_owner),
            handoff_complete: false,
        })
    }

    pub(crate) fn start_initializing(
        &self,
        control_state: ControlPlaneState,
        shutdown: Option<Arc<AtomicBool>>,
    ) -> Result<InitializingServer, DaemonFatalError> {
        let listener = self
            .listener
            .as_ref()
            .expect("bound listener is live")
            .try_clone()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
        let auth_token = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .auth_token()
            .to_owned();
        let publication_revoker = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .publication_revoker();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let terminal = Arc::new(Mutex::new(None));
        let thread_terminal = Arc::clone(&terminal);
        let join = thread::spawn(move || {
            let result = run_control_loop(
                listener,
                control_state,
                auth_token,
                ControlLoopConfig {
                    handoff: Some(thread_stop),
                    shutdown,
                    request_limit: usize::MAX,
                    publication_revoker,
                },
            )
            .map(|_| ());
            *thread_terminal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
            result
        });
        Ok(InitializingServer {
            stop,
            terminal,
            join,
        })
    }

    pub(crate) fn finish_initializing(
        &mut self,
        initializing: InitializingServer,
    ) -> Result<(), DaemonFatalError> {
        initializing.stop()?;
        self.handoff_complete = true;
        Ok(())
    }

    pub(crate) fn serve_control_only(
        mut self,
        control_state: ControlPlaneState,
        shutdown: Option<&Arc<AtomicBool>>,
        max_requests: Option<usize>,
    ) -> Result<(), DaemonFatalError> {
        if !self.handoff_complete {
            self.withdraw();
            return Err(DaemonFatalError::ControlPlaneFailure);
        }
        let listener = self.listener.take().expect("bound listener is live");
        let auth_token = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .auth_token()
            .to_owned();
        let publication_revoker = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .publication_revoker();
        let result = run_control_loop(
            listener,
            control_state,
            auth_token,
            ControlLoopConfig {
                handoff: None,
                shutdown: shutdown.cloned(),
                request_limit: max_requests.unwrap_or(usize::MAX),
                publication_revoker,
            },
        );
        self.withdraw();
        result.map(|_| ())
    }

    pub(crate) fn serve(mut self, mut context: Context<'_>) -> Result<(), DaemonFatalError> {
        if !self.handoff_complete {
            self.withdraw();
            return Err(DaemonFatalError::ControlPlaneFailure);
        }
        let mut control_publisher = context
            .control_publisher
            .take()
            .expect("server owns the control-plane publisher");
        let summary = match context.owned_store.status_summary() {
            Ok(summary) => summary,
            Err(_) => {
                control_publisher.mark_blocked_with_runtimes(
                    super::CoreReason::MetadataUnavailable,
                    context.control_state.snapshot().runtimes,
                )?;
                return self.serve_control_only(
                    context.control_state,
                    context.shutdown,
                    context.max_requests,
                );
            }
        };
        let query_service = match start_or_block_core_service(
            &mut control_publisher,
            context.control_state.snapshot().runtimes,
            || {
                SearchService::start(
                    context.data_dir,
                    context.search_runtime_config.clone(),
                    context.artifact_fault_reporter.clone(),
                )
            },
        )? {
            Some(service) => service,
            None => {
                return self.serve_control_only(
                    context.control_state,
                    context.shutdown,
                    context.max_requests,
                );
            }
        };
        if announce_ready(&summary).is_err()
            || control_publisher.publish_prepared_serving().is_err()
        {
            self.withdraw();
            query_service.abort_for_process_exit();
            return Err(DaemonFatalError::ControlPlaneFailure);
        }
        let status_updater = StatusUpdater::start(
            context.data_dir,
            control_publisher,
            context.search_runtime_config.resident_embedding.clone(),
            context.control_state.snapshot().runtimes,
            context.runtime_health_receiver.take(),
        );
        let terminal = self.serve_listener(&context, &query_service, &status_updater);
        self.withdraw_then_finish(
            terminal,
            ServerCleanup {
                updater: status_updater,
                service: query_service,
                shutdown_updater: StatusUpdater::shutdown,
                drain: |query_service: SearchService| {
                    query_service
                        .drain_admitted()
                        .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))
                },
                cancel_and_join: |query_service: SearchService| {
                    query_service
                        .shutdown()
                        .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))
                },
                abort: SearchService::abort_for_process_exit,
            },
        )
    }

    fn withdraw_then_finish<Updater, Service, ShutdownUpdater, Drain, CancelAndJoin, Abort>(
        mut self,
        terminal: ServerStop,
        cleanup: ServerCleanup<Updater, Service, ShutdownUpdater, Drain, CancelAndJoin, Abort>,
    ) -> Result<(), DaemonFatalError>
    where
        ShutdownUpdater: FnOnce(Updater) -> Result<(), DaemonFatalError>,
        Drain: FnOnce(Service) -> Result<(), DaemonFatalError>,
        CancelAndJoin: FnOnce(Service) -> Result<(), DaemonFatalError>,
        Abort: FnOnce(Service),
    {
        let ServerCleanup {
            updater,
            service,
            shutdown_updater,
            drain,
            cancel_and_join,
            abort,
        } = cleanup;
        // Discovery is a lease on the live listener, not on background data
        // services. Revoke it before any potentially blocking service join.
        self.withdraw();
        let (service_result, updater_result) = match terminal {
            ServerStop::RequestLimitReached => {
                // A request limit is an orderly bounded exit. Quiesce cached
                // status publication before draining admitted data-plane work.
                let updater_result = shutdown_updater(updater);
                let service_result = drain(service);
                (service_result, updater_result)
            }
            ServerStop::ParentShutdown => {
                // A status refresh can be waiting on metadata while the data
                // plane owns a write transaction. Cancel that work before
                // joining the updater so shutdown does not spend its watchdog
                // grace with mutation workers still live.
                let service_result = cancel_and_join(service);
                let updater_result = shutdown_updater(updater);
                (service_result, updater_result)
            }
            ServerStop::Fatal(fatal) => {
                abort(service);
                (Err(fatal), shutdown_updater(updater))
            }
        };
        match (updater_result, service_result) {
            (Err(error), _) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn serve_listener(
        &mut self,
        context: &Context<'_>,
        query_service: &SearchService,
        status_updater: &StatusUpdater,
    ) -> ServerStop {
        let mut listener = Some(self.listener.take().expect("bound listener is live"));
        let auth_token = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .auth_token()
            .to_owned();
        let publication_revoker = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .publication_revoker();
        let request_limit = context.max_requests.unwrap_or(usize::MAX);
        let mut handled_requests = 0;
        while handled_requests < request_limit {
            match observe_runtime(context, query_service, status_updater) {
                RuntimeEvent::Running => {}
                RuntimeEvent::ShutdownRequested => {
                    drop(listener.take());
                    self.withdraw();
                    return ServerStop::ParentShutdown;
                }
                event => {
                    drop(listener.take());
                    self.withdraw();
                    return ServerStop::Fatal(runtime_failure(event));
                }
            }

            match listener
                .as_ref()
                .expect("business listener is live")
                .accept()
            {
                Ok((stream, _)) => {
                    let result = handle_business_with_watchdog(
                        stream,
                        context.shutdown.cloned(),
                        publication_revoker.clone(),
                        |stream| {
                            let _ = connection::handle(
                                stream,
                                connection::Context {
                                    store: context.store,
                                    owned_store: context.owned_store,
                                    query_service,
                                    processing_contract: context.processing_contract,
                                    auth_token: &auth_token,
                                    control_state: &context.control_state,
                                },
                            );
                        },
                    );
                    if let Err(error) = result {
                        drop(listener.take());
                        self.withdraw();
                        return ServerStop::Fatal(error);
                    }
                    handled_requests += 1;
                }
                Err(error) => match classify_accept_error(error.kind()) {
                    AcceptErrorDisposition::NoConnectionReady
                    | AcceptErrorDisposition::ConnectionLocal => {
                        thread::sleep(LISTENER_POLL_INTERVAL);
                    }
                    AcceptErrorDisposition::ListenerFatal => {
                        drop(listener.take());
                        self.withdraw();
                        return ServerStop::Fatal(DaemonFatalError::ControlPlaneFailure);
                    }
                },
            }
        }
        ServerStop::RequestLimitReached
    }

    fn withdraw(&mut self) {
        drop(self.listener.take());
        drop(self.daemon_owner.take());
    }
}

struct ServerCleanup<Updater, Service, ShutdownUpdater, Drain, CancelAndJoin, Abort> {
    updater: Updater,
    service: Service,
    shutdown_updater: ShutdownUpdater,
    drain: Drain,
    cancel_and_join: CancelAndJoin,
    abort: Abort,
}

fn start_or_block_core_service<Service, Error>(
    publisher: &mut ControlPlanePublisher,
    runtimes: super::OptionalRuntimeMatrix,
    start: impl FnOnce() -> Result<Service, Error>,
) -> Result<Option<Service>, DaemonFatalError> {
    match start() {
        Ok(service) => Ok(Some(service)),
        Err(_) => {
            publisher
                .mark_blocked_with_runtimes(super::CoreReason::ArtifactUnavailable, runtimes)?;
            Ok(None)
        }
    }
}

fn announce_ready(summary: &StoreStatusSummary) -> Result<(), DaemonFatalError> {
    println!("resume-daemon foreground ready");
    println!("mode: foreground");
    println!(
        "index health: {}",
        crate::index_health_label(summary.index_health)
    );
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    io::stdout()
        .flush()
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)
}

pub(crate) struct InitializingServer {
    stop: Arc<AtomicBool>,
    terminal: Arc<Mutex<Option<Result<(), DaemonFatalError>>>>,
    join: JoinHandle<Result<(), DaemonFatalError>>,
}

impl InitializingServer {
    pub(crate) fn check_health(&self) -> Result<(), DaemonFatalError> {
        if !self.join.is_finished() {
            return Ok(());
        }
        self.terminal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unwrap_or(Err(DaemonFatalError::ControlPlaneFailure))
    }

    pub(crate) fn stop(self) -> Result<(), DaemonFatalError> {
        self.stop.store(true, Ordering::Release);
        self.join
            .join()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)??;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcceptErrorDisposition {
    NoConnectionReady,
    ConnectionLocal,
    ListenerFatal,
}

fn classify_accept_error(kind: io::ErrorKind) -> AcceptErrorDisposition {
    match kind {
        io::ErrorKind::WouldBlock => AcceptErrorDisposition::NoConnectionReady,
        io::ErrorKind::Interrupted
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::TimedOut => AcceptErrorDisposition::ConnectionLocal,
        _ => AcceptErrorDisposition::ListenerFatal,
    }
}

/// Runs the IPC control plane. Its fatal channel is structurally limited to
/// listener ownership/integrity failures and closed supervised runtime events.
fn bind(
    addr: SocketAddr,
    daemon_owner: &DaemonGenerationOwner,
) -> Result<TcpListener, DaemonFatalError> {
    let listener = TcpListener::bind(addr).map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
    daemon_owner
        .publish(local_addr)
        .map_err(|error| match error {
            GenerationError::RuntimeIntegrity => DaemonFatalError::RuntimeIntegrity,
            GenerationError::Storage => DaemonFatalError::ControlPlaneFailure,
        })?;
    println!("ipc status endpoint: http://{local_addr}/status");
    io::stdout()
        .flush()
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
    Ok(listener)
}

fn observe_runtime(
    context: &Context<'_>,
    query_service: &SearchService,
    status_updater: &StatusUpdater,
) -> RuntimeEvent {
    if context
        .shutdown
        .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
    {
        return RuntimeEvent::ShutdownRequested;
    }
    if query_service.check_health().is_err() {
        return RuntimeEvent::QueryWorkerStopped;
    }
    if status_updater.check_health().is_err() {
        return RuntimeEvent::StatusUpdaterStopped;
    }
    let Some(receiver) = context.worker_result_receiver else {
        return RuntimeEvent::Running;
    };
    match receiver.try_recv() {
        Ok(Ok(()))
            if context
                .shutdown
                .is_some_and(|shutdown| shutdown.load(Ordering::Acquire)) =>
        {
            RuntimeEvent::ShutdownRequested
        }
        Ok(Ok(())) => RuntimeEvent::ImportWorkerStopped,
        Ok(Err(error)) => RuntimeEvent::ImportWorkerFailed(error),
        Err(TryRecvError::Disconnected) => RuntimeEvent::ImportWorkerStopped,
        Err(TryRecvError::Empty) => RuntimeEvent::Running,
    }
}

fn fatal_for_event(event: RuntimeEvent) -> Result<(), DaemonFatalError> {
    match event {
        RuntimeEvent::Running | RuntimeEvent::ShutdownRequested => Ok(()),
        RuntimeEvent::ImportWorkerStopped
        | RuntimeEvent::QueryWorkerStopped
        | RuntimeEvent::StatusUpdaterStopped => Err(DaemonFatalError::ControlPlaneFailure),
        RuntimeEvent::ImportWorkerFailed(error) => Err(error),
    }
}

fn runtime_failure(event: RuntimeEvent) -> DaemonFatalError {
    fatal_for_event(event).expect_err("only terminal runtime events are passed to runtime_failure")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    use std::thread;
    use std::time::Duration;

    use meta_store::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease};

    use super::{
        classify_accept_error, fatal_for_event, start_or_block_core_service,
        AcceptErrorDisposition, BoundServer, ServerStop,
    };
    use crate::ipc::{
        ControlPlaneState, CoreHealth, CoreReason, CoreState, DaemonFatalError,
        DaemonGenerationOwner, OptionalRuntimeHealth, OptionalRuntimeMatrix, OptionalRuntimeReason,
        OwnerMode, RuntimeEvent,
    };

    #[test]
    fn pending_connection_accept_failures_never_become_daemon_fatal() {
        for kind in [
            io::ErrorKind::Interrupted,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::TimedOut,
        ] {
            assert_eq!(
                classify_accept_error(kind),
                AcceptErrorDisposition::ConnectionLocal
            );
        }
        assert_eq!(
            classify_accept_error(io::ErrorKind::WouldBlock),
            AcceptErrorDisposition::NoConnectionReady
        );
        assert_eq!(
            classify_accept_error(io::ErrorKind::PermissionDenied),
            AcceptErrorDisposition::ListenerFatal
        );
    }

    #[test]
    fn closed_runtime_events_have_deterministic_terminal_classification() {
        assert_eq!(fatal_for_event(RuntimeEvent::Running), Ok(()));
        assert_eq!(fatal_for_event(RuntimeEvent::ShutdownRequested), Ok(()));
        assert_eq!(
            fatal_for_event(RuntimeEvent::QueryWorkerStopped),
            Err(DaemonFatalError::ControlPlaneFailure)
        );
        assert_eq!(
            fatal_for_event(RuntimeEvent::StatusUpdaterStopped),
            Err(DaemonFatalError::ControlPlaneFailure)
        );
        assert_eq!(
            fatal_for_event(RuntimeEvent::ImportWorkerStopped),
            Err(DaemonFatalError::ControlPlaneFailure)
        );
        assert_eq!(
            fatal_for_event(RuntimeEvent::ImportWorkerFailed(
                DaemonFatalError::RuntimeIntegrity
            )),
            Err(DaemonFatalError::RuntimeIntegrity)
        );
    }

    #[test]
    fn query_service_start_failure_keeps_an_artifact_blocked_control_snapshot() {
        let (state, mut publisher) = ControlPlaneState::initializing();
        let runtimes = OptionalRuntimeMatrix {
            embedding: OptionalRuntimeHealth::available(),
            ocr: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Invalid),
            classifier: OptionalRuntimeHealth::available(),
        };
        publisher.set_runtimes(runtimes).unwrap();

        let service = start_or_block_core_service(&mut publisher, runtimes, || {
            Err::<(), _>("synthetic query start failure")
        })
        .unwrap();

        assert!(service.is_none());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.core.state, CoreState::Blocked);
        assert_eq!(snapshot.core.reason, Some(CoreReason::ArtifactUnavailable));
        assert_eq!(snapshot.runtimes, runtimes);
        assert_eq!(
            snapshot.capabilities.keyword_search.state.label(),
            "blocked"
        );
    }

    #[test]
    fn parent_shutdown_withdraws_generation_and_cancels_data_plane_before_status_updater_join() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory_owner =
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            };
        let generation = DaemonGenerationOwner::acquire(
            std::sync::Arc::new(data_directory_owner),
            OwnerMode::Standalone,
            "a".repeat(64),
        )
        .unwrap();
        let server =
            BoundServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap(), generation).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("ipc.endpoints.json")).unwrap())
                .unwrap();
        let addr = manifest["status"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/status")
            .unwrap()
            .parse::<SocketAddr>()
            .unwrap();
        let (shutdown_entered_sender, shutdown_entered_receiver) = mpsc::sync_channel(1);
        let (shutdown_release_sender, shutdown_release_receiver) = mpsc::sync_channel(1);
        let (service_cancelled_sender, service_cancelled_receiver) = mpsc::sync_channel(1);

        thread::scope(|scope| {
            let join = scope.spawn(move || {
                server.withdraw_then_finish(
                    ServerStop::ParentShutdown,
                    super::ServerCleanup {
                        updater: (),
                        service: (),
                        shutdown_updater: |()| {
                            shutdown_entered_sender.send(()).unwrap();
                            shutdown_release_receiver.recv().unwrap();
                            Ok(())
                        },
                        drain: |()| panic!("parent shutdown must not use request-limit drain"),
                        cancel_and_join: |()| {
                            service_cancelled_sender.send(()).unwrap();
                            Ok(())
                        },
                        abort: |()| panic!("normal terminal must not use fatal abort"),
                    },
                )
            });
            let service_cancelled_before_updater = service_cancelled_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_ok();
            shutdown_entered_receiver.recv().unwrap();
            assert!(!directory.path().join("ipc.endpoints.json").exists());
            assert!(!directory.path().join("ipc.auth").exists());
            assert!(TcpStream::connect(addr).is_err());
            shutdown_release_sender.send(()).unwrap();
            assert_eq!(join.join().unwrap(), Ok(()));
            assert!(
                service_cancelled_before_updater,
                "parent shutdown waited for the status updater before cancelling the data plane"
            );
        });
    }

    #[test]
    fn request_limit_stops_status_updater_before_draining_data_plane() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory_owner = Arc::new(
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            },
        );
        let generation = DaemonGenerationOwner::acquire(
            data_directory_owner,
            OwnerMode::Standalone,
            "b".repeat(64),
        )
        .unwrap();
        let server =
            BoundServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap(), generation).unwrap();
        let (updater_entered_sender, updater_entered_receiver) = mpsc::sync_channel(1);
        let (updater_release_sender, updater_release_receiver) = mpsc::sync_channel(1);
        let (service_drained_sender, service_drained_receiver) = mpsc::sync_channel(1);

        thread::scope(|scope| {
            let join = scope.spawn(move || {
                server.withdraw_then_finish(
                    ServerStop::RequestLimitReached,
                    super::ServerCleanup {
                        updater: (),
                        service: (),
                        shutdown_updater: |()| {
                            updater_entered_sender.send(()).unwrap();
                            updater_release_receiver.recv().unwrap();
                            Ok(())
                        },
                        drain: |()| {
                            service_drained_sender.send(()).unwrap();
                            Ok(())
                        },
                        cancel_and_join: |()| {
                            panic!("request-limit exit must not cancel admitted work")
                        },
                        abort: |()| panic!("normal terminal must not use fatal abort"),
                    },
                )
            });
            updater_entered_receiver.recv().unwrap();
            assert!(!directory.path().join("ipc.endpoints.json").exists());
            assert!(!directory.path().join("ipc.auth").exists());
            let service_drained_before_updater_stopped = service_drained_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_ok();
            updater_release_sender.send(()).unwrap();
            assert_eq!(join.join().unwrap(), Ok(()));
            if !service_drained_before_updater_stopped {
                service_drained_receiver.recv().unwrap();
            }
            assert!(
                !service_drained_before_updater_stopped,
                "request-limit exit drained the data plane before the status updater stopped"
            );
        });
    }

    #[test]
    fn parent_shutdown_revokes_initializing_discovery_before_bootstrap_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory_owner = Arc::new(
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            },
        );
        let generation = DaemonGenerationOwner::acquire(
            data_directory_owner,
            OwnerMode::DesktopSupervised,
            "d".repeat(64),
        )
        .unwrap();
        let server =
            BoundServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap(), generation).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("ipc.endpoints.json")).unwrap())
                .unwrap();
        let address = manifest["status"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/status")
            .unwrap();
        let (state, _publisher) = ControlPlaneState::initializing();
        let shutdown = Arc::new(AtomicBool::new(false));
        let initializing = server
            .start_initializing(state, Some(Arc::clone(&shutdown)))
            .unwrap();
        assert!(directory.path().join("ipc.endpoints.json").exists());
        let accepted_before = crate::ipc::process_metrics().snapshot().accepted;
        let mut stalled = TcpStream::connect(address).unwrap();
        stalled.write_all(b"G").unwrap();
        let accepted_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while crate::ipc::process_metrics().snapshot().accepted == accepted_before
            && std::time::Instant::now() < accepted_deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            crate::ipc::process_metrics().snapshot().accepted > accepted_before,
            "initializing server did not admit the stalled connection"
        );

        shutdown.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while (directory.path().join("ipc.endpoints.json").exists()
            || directory.path().join("ipc.auth").exists())
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        assert!(!directory.path().join("ipc.endpoints.json").exists());
        assert!(!directory.path().join("ipc.auth").exists());
        drop(stalled);
        initializing.stop().unwrap();
        drop(server);
    }

    #[test]
    fn control_only_owner_never_returns_404_if_ready_is_published_out_of_order() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory_owner = Arc::new(
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            },
        );
        let generation = DaemonGenerationOwner::acquire(
            data_directory_owner,
            OwnerMode::DesktopSupervised,
            "e".repeat(64),
        )
        .unwrap();
        let server =
            BoundServer::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap(), generation).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("ipc.endpoints.json")).unwrap())
                .unwrap();
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("ipc.auth")).unwrap()).unwrap();
        let address = manifest["status"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/status")
            .unwrap();
        let token = auth["token"].as_str().unwrap();
        let (state, mut publisher) = ControlPlaneState::initializing();
        let initializing = server.start_initializing(state, None).unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        thread::sleep(Duration::from_millis(75));

        let unavailable = OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::NotConfigured);
        publisher
            .publish_without_store_for_test(
                CoreHealth {
                    state: CoreState::Ready,
                    reason: None,
                },
                OptionalRuntimeMatrix {
                    embedding: unavailable,
                    ocr: unavailable,
                    classifier: unavailable,
                },
            )
            .unwrap();
        write!(
            stream,
            "POST /search HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        assert!(!response.starts_with("HTTP/1.1 404"), "{response}");
        assert!(response.contains("\"schema_version\":\"resume-ir.error.v2\""));
        initializing.stop().unwrap();
        drop(server);
    }
}
