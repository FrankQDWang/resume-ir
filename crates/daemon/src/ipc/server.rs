use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::search_service::SearchService;
use super::{connection, DaemonFatalError, DaemonGenerationOwner, GenerationError, RuntimeEvent};
use crate::search_runtime_config::SearchRuntimeConfig;

pub(crate) struct Context<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) store: &'a ReadMetaStore,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) addr: SocketAddr,
    pub(crate) max_requests: Option<usize>,
    pub(crate) search_runtime_config: SearchRuntimeConfig,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) shutdown: Option<&'a Arc<AtomicBool>>,
    pub(crate) worker_result_receiver: Option<&'a Receiver<Result<(), DaemonFatalError>>>,
    pub(crate) daemon_owner: &'a DaemonGenerationOwner<'a>,
}

/// Runs the IPC control plane. Its fatal channel is structurally limited to
/// listener ownership/integrity failures and closed supervised runtime events.
pub(crate) fn serve(context: Context<'_>) -> Result<(), DaemonFatalError> {
    let listener = bind(context.addr, context.daemon_owner)?;
    let query_service =
        SearchService::start(context.data_dir, context.search_runtime_config.clone())
            .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))?;
    let request_limit = context.max_requests.unwrap_or(usize::MAX);
    let mut handled_requests = 0_usize;

    while handled_requests < request_limit {
        match observe_runtime(&context, &query_service) {
            RuntimeEvent::Running => {}
            RuntimeEvent::ShutdownRequested => break,
            event => return Err(runtime_failure(event)),
        }

        match listener.accept() {
            Ok((stream, _)) => {
                let _ = connection::handle(
                    stream,
                    connection::Context {
                        store: context.store,
                        owned_store: context.owned_store,
                        query_service: &query_service,
                        processing_contract: context.processing_contract,
                        daemon_owner: context.daemon_owner,
                    },
                );
                handled_requests += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err(DaemonFatalError::ControlPlaneFailure),
        }
    }

    query_service
        .finish()
        .map_err(|_| runtime_failure(RuntimeEvent::QueryWorkerStopped))
}

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
    use super::fatal_for_event;
    use crate::ipc::{DaemonFatalError, RuntimeEvent};

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
}
