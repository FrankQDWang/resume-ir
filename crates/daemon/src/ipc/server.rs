use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::search_service::{ArtifactFaultReporter, SearchService};
use super::{connection, DaemonFatalError, DaemonGenerationOwner, GenerationError, RuntimeEvent};
use crate::search_runtime_config::SearchRuntimeConfig;

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
}

/// A listener whose generation credentials and endpoint manifest have already
/// been published. Background data services must not start until this control
/// plane capability exists.
pub(crate) struct BoundServer<'owner> {
    listener: Option<TcpListener>,
    daemon_owner: Option<DaemonGenerationOwner<'owner>>,
}

impl<'owner> BoundServer<'owner> {
    pub(crate) fn bind(
        addr: SocketAddr,
        daemon_owner: DaemonGenerationOwner<'owner>,
    ) -> Result<Self, DaemonFatalError> {
        let listener = bind(addr, &daemon_owner)?;
        Ok(Self {
            listener: Some(listener),
            daemon_owner: Some(daemon_owner),
        })
    }

    pub(crate) fn serve(mut self, context: Context<'_>) -> Result<(), DaemonFatalError> {
        let query_service = match SearchService::start(
            context.data_dir,
            context.search_runtime_config.clone(),
            context.artifact_fault_reporter.clone(),
        ) {
            Ok(service) => service,
            Err(_) => {
                self.withdraw();
                return Err(runtime_failure(RuntimeEvent::QueryWorkerStopped));
            }
        };
        let terminal = self.serve_listener(&context, &query_service);
        self.withdraw_then_finish(
            terminal,
            query_service,
            |query_service| {
                query_service
                    .drain_admitted()
                    .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))
            },
            |query_service| {
                query_service
                    .shutdown()
                    .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))
            },
            SearchService::abort_for_process_exit,
        )
    }

    fn withdraw_then_finish<Service>(
        mut self,
        terminal: ServerStop,
        service: Service,
        drain: impl FnOnce(Service) -> Result<(), DaemonFatalError>,
        cancel_and_join: impl FnOnce(Service) -> Result<(), DaemonFatalError>,
        abort: impl FnOnce(Service),
    ) -> Result<(), DaemonFatalError> {
        // Discovery is a lease on the live listener, not on background data
        // services. Revoke it before any potentially blocking service join.
        self.withdraw();
        match terminal {
            ServerStop::RequestLimitReached => drain(service),
            ServerStop::ParentShutdown => cancel_and_join(service),
            ServerStop::Fatal(fatal) => {
                abort(service);
                Err(fatal)
            }
        }
    }

    fn serve_listener(&self, context: &Context<'_>, query_service: &SearchService) -> ServerStop {
        let listener = self.listener.as_ref().expect("bound listener is live");
        let auth_token = self
            .daemon_owner
            .as_ref()
            .expect("bound generation is published")
            .auth_token();
        let request_limit = context.max_requests.unwrap_or(usize::MAX);
        let mut handled_requests = 0_usize;

        while handled_requests < request_limit {
            match observe_runtime(context, query_service) {
                RuntimeEvent::Running => {}
                RuntimeEvent::ShutdownRequested => return ServerStop::ParentShutdown,
                event => return ServerStop::Fatal(runtime_failure(event)),
            }

            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = connection::handle(
                        stream,
                        connection::Context {
                            store: context.store,
                            owned_store: context.owned_store,
                            query_service,
                            processing_contract: context.processing_contract,
                            auth_token,
                        },
                    );
                    handled_requests += 1;
                }
                Err(error) => match classify_accept_error(error.kind()) {
                    AcceptErrorDisposition::NoConnectionReady
                    | AcceptErrorDisposition::ConnectionLocal => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    AcceptErrorDisposition::ListenerFatal => {
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
    daemon_owner: &DaemonGenerationOwner<'_>,
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

fn observe_runtime(context: &Context<'_>, query_service: &SearchService) -> RuntimeEvent {
    if context
        .shutdown
        .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
    {
        return RuntimeEvent::ShutdownRequested;
    }
    if query_service.check_health().is_err() {
        return RuntimeEvent::QueryWorkerStopped;
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
        RuntimeEvent::ImportWorkerStopped | RuntimeEvent::QueryWorkerStopped => {
            Err(DaemonFatalError::ControlPlaneFailure)
        }
        RuntimeEvent::ImportWorkerFailed(error) => Err(error),
    }
}

fn runtime_failure(event: RuntimeEvent) -> DaemonFatalError {
    fatal_for_event(event).expect_err("only terminal runtime events are passed to runtime_failure")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::net::{SocketAddr, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    use meta_store::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease};

    use super::{
        classify_accept_error, fatal_for_event, AcceptErrorDisposition, BoundServer, ServerStop,
    };
    use crate::ipc::{DaemonFatalError, DaemonGenerationOwner, OwnerMode, RuntimeEvent};

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
    fn published_generation_is_withdrawn_before_service_shutdown_join() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory_owner =
            match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
                DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
                DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
            };
        let generation =
            DaemonGenerationOwner::acquire(&data_directory_owner, OwnerMode::Standalone).unwrap();
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

        thread::scope(|scope| {
            let join = scope.spawn(move || {
                server.withdraw_then_finish(
                    ServerStop::ParentShutdown,
                    (),
                    |()| panic!("parent shutdown must not use request-limit drain"),
                    |()| {
                        shutdown_entered_sender.send(()).unwrap();
                        shutdown_release_receiver.recv().unwrap();
                        Ok(())
                    },
                    |()| panic!("normal terminal must not use fatal abort"),
                )
            });
            shutdown_entered_receiver.recv().unwrap();
            assert!(!directory.path().join("ipc.endpoints.json").exists());
            assert!(!directory.path().join("ipc.auth").exists());
            assert!(TcpStream::connect(addr).is_err());
            shutdown_release_sender.send(()).unwrap();
            assert_eq!(join.join().unwrap(), Ok(()));
        });
    }
}
