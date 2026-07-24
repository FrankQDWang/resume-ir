use std::io::Read;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::ipc::generation::GenerationPublicationRevoker;
use crate::ipc::{connection, ControlPlaneState, DaemonFatalError};

use super::{classify_accept_error, AcceptErrorDisposition};

const CONNECTION_HARD_DEADLINE: Duration = Duration::from_secs(5);
const RESPONSE_DELIVERY_RECEIPT_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const LISTENER_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy)]
struct BusinessConnectionTiming {
    hard_deadline: Duration,
    delivery_receipt_timeout: Duration,
    poll_interval: Duration,
}

const BUSINESS_CONNECTION_TIMING: BusinessConnectionTiming = BusinessConnectionTiming {
    hard_deadline: CONNECTION_HARD_DEADLINE,
    delivery_receipt_timeout: RESPONSE_DELIVERY_RECEIPT_TIMEOUT,
    poll_interval: LISTENER_POLL_INTERVAL,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ControlLoopStop {
    Handoff,
    ParentShutdown,
    RequestLimitReached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BusinessConnectionFinish {
    Immediate,
    AwaitResponseDelivery,
}

pub(super) struct ControlLoopConfig {
    pub(super) handoff: Option<Arc<AtomicBool>>,
    pub(super) shutdown: Option<Arc<AtomicBool>>,
    pub(super) request_limit: usize,
    pub(super) publication_revoker: GenerationPublicationRevoker,
}

struct ActiveControlConnection<'scope> {
    cancellation: TcpStream,
    join: thread::ScopedJoinHandle<'scope, ()>,
    deadline: Instant,
    cancellation_sent: bool,
}

impl ActiveControlConnection<'_> {
    fn cancel(&mut self) {
        if !self.cancellation_sent {
            let _ = self.cancellation.shutdown(Shutdown::Both);
            self.cancellation_sent = true;
        }
    }

    fn cancel_if_expired(&mut self) {
        if Instant::now() >= self.deadline {
            self.cancel();
        }
    }

    fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    fn join(self) -> Result<(), DaemonFatalError> {
        self.join
            .join()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)
    }
}

pub(super) fn run_control_loop(
    listener: TcpListener,
    state: ControlPlaneState,
    auth_token: String,
    config: ControlLoopConfig,
) -> Result<ControlLoopStop, DaemonFatalError> {
    thread::scope(|scope| {
        let mut active = None;
        let mut handled = 0;
        loop {
            // Revocation must win a simultaneous bootstrap handoff so the
            // parent watchdog never observes a published dead generation.
            if config
                .shutdown
                .as_ref()
                .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
            {
                config.publication_revoker.withdraw();
                cancel_and_join(&mut active)?;
                return Ok(ControlLoopStop::ParentShutdown);
            }
            if config
                .handoff
                .as_ref()
                .is_some_and(|handoff| handoff.load(Ordering::Acquire))
            {
                cancel_and_join(&mut active)?;
                return Ok(ControlLoopStop::Handoff);
            }
            if reap_finished(&mut active)? {
                handled += 1;
            }
            if handled >= config.request_limit {
                return Ok(ControlLoopStop::RequestLimitReached);
            }
            if let Some(connection) = active.as_mut() {
                connection.cancel_if_expired();
                thread::sleep(LISTENER_POLL_INTERVAL);
                continue;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let cancellation = match stream.try_clone() {
                        Ok(cancellation) => cancellation,
                        Err(_) => {
                            handled += 1;
                            continue;
                        }
                    };
                    let state = &state;
                    let token = auth_token.as_str();
                    active = Some(ActiveControlConnection {
                        cancellation,
                        join: scope.spawn(move || {
                            let _ = connection::handle_control(stream, state, token);
                        }),
                        deadline: Instant::now()
                            .checked_add(CONNECTION_HARD_DEADLINE)
                            .unwrap_or_else(Instant::now),
                        cancellation_sent: false,
                    });
                }
                Err(error) => match classify_accept_error(error.kind()) {
                    AcceptErrorDisposition::NoConnectionReady
                    | AcceptErrorDisposition::ConnectionLocal => {
                        thread::sleep(LISTENER_POLL_INTERVAL);
                    }
                    AcceptErrorDisposition::ListenerFatal => {
                        config.publication_revoker.withdraw();
                        cancel_and_join(&mut active)?;
                        return Err(DaemonFatalError::ControlPlaneFailure);
                    }
                },
            }
        }
    })
}

pub(super) fn handle_business_with_watchdog(
    stream: TcpStream,
    shutdown: Option<Arc<AtomicBool>>,
    publication_revoker: GenerationPublicationRevoker,
    finish: BusinessConnectionFinish,
    handle: impl FnOnce(TcpStream) -> crate::ipc::ConnectionCompletion,
) -> Result<(), DaemonFatalError> {
    handle_business_with_timing(
        stream,
        shutdown,
        publication_revoker,
        finish,
        BUSINESS_CONNECTION_TIMING,
        handle,
    )
}

fn handle_business_with_timing(
    stream: TcpStream,
    shutdown: Option<Arc<AtomicBool>>,
    publication_revoker: GenerationPublicationRevoker,
    finish: BusinessConnectionFinish,
    timing: BusinessConnectionTiming,
    handle: impl FnOnce(TcpStream) -> crate::ipc::ConnectionCompletion,
) -> Result<(), DaemonFatalError> {
    let cancellation = match stream.try_clone() {
        Ok(cancellation) => cancellation,
        Err(error) => {
            if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
                eprintln!(
                    "[DEBUG-s49-reset] cancellation_clone_failed kind={:?}",
                    error.kind()
                );
            }
            return Ok(());
        }
    };
    let delivery_receipt = matches!(finish, BusinessConnectionFinish::AwaitResponseDelivery)
        .then(|| stream.try_clone().ok())
        .flatten();
    let finished = Arc::new(AtomicBool::new(false));
    let watcher_finished = Arc::clone(&finished);
    let cancelled = Arc::new(AtomicBool::new(false));
    let watcher_cancelled = Arc::clone(&cancelled);
    let watchdog = thread::Builder::new()
        .name("resume-ir-ipc-watchdog".to_string())
        .spawn(move || {
            let deadline = Instant::now()
                .checked_add(timing.hard_deadline)
                .unwrap_or_else(Instant::now);
            loop {
                if watcher_finished.load(Ordering::Acquire) {
                    return;
                }
                if shutdown
                    .as_ref()
                    .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
                {
                    if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
                        eprintln!("[DEBUG-s49-reset] watchdog_cancel reason=parent_shutdown");
                    }
                    publication_revoker.withdraw();
                    watcher_cancelled.store(true, Ordering::Release);
                    let _ = cancellation.shutdown(Shutdown::Both);
                    return;
                }
                if Instant::now() >= deadline {
                    if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
                        eprintln!("[DEBUG-s49-reset] watchdog_cancel reason=hard_deadline");
                    }
                    watcher_cancelled.store(true, Ordering::Release);
                    let _ = cancellation.shutdown(Shutdown::Both);
                    return;
                }
                thread::sleep(timing.poll_interval);
            }
        })
        .map_err(|error| {
            if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
                eprintln!(
                    "[DEBUG-s49-reset] watchdog_spawn_failed kind={:?}",
                    error.kind()
                );
            }
            DaemonFatalError::ControlPlaneFailure
        })?;

    let completion = handle(stream);
    if matches!(finish, BusinessConnectionFinish::AwaitResponseDelivery) {
        while !completion.is_finished() && !cancelled.load(Ordering::Acquire) {
            thread::sleep(timing.poll_interval);
        }
    }
    finished.store(true, Ordering::Release);
    watchdog
        .join()
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
    if let (true, false, Some(mut delivery_receipt)) = (
        completion.is_finished(),
        cancelled.load(Ordering::Acquire),
        delivery_receipt,
    ) {
        let _ = delivery_receipt.set_read_timeout(Some(timing.delivery_receipt_timeout));
        loop {
            match delivery_receipt.read(&mut [0_u8; 1]) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
    Ok(())
}

fn reap_finished(
    active: &mut Option<ActiveControlConnection<'_>>,
) -> Result<bool, DaemonFatalError> {
    if !active
        .as_ref()
        .is_some_and(ActiveControlConnection::is_finished)
    {
        return Ok(false);
    }
    active.take().expect("finished connection exists").join()?;
    Ok(true)
}

fn cancel_and_join(
    active: &mut Option<ActiveControlConnection<'_>>,
) -> Result<(), DaemonFatalError> {
    let Some(mut connection) = active.take() else {
        return Ok(());
    };
    connection.cancel();
    connection.join()
}

#[cfg(test)]
mod tests {
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    use meta_store::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease};

    use super::{
        handle_business_with_timing, handle_business_with_watchdog, BusinessConnectionFinish,
        BusinessConnectionTiming,
    };
    use crate::ipc::generation::{DaemonGenerationOwner, OwnerMode};
    use crate::ipc::{ConnectionCompletion, ConnectionOutcome};

    #[test]
    fn final_connection_waits_for_response_completion_before_delivery_receipt() {
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
            "e".repeat(64),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let (handler_returned_sender, handler_returned_receiver) = mpsc::sync_channel(1);
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
        let revoker = generation.publication_revoker();
        let completion = ConnectionCompletion::accepted();
        let response_owner = completion.defer();

        let join = thread::spawn(move || {
            let result = handle_business_with_watchdog(
                server,
                None,
                revoker,
                BusinessConnectionFinish::AwaitResponseDelivery,
                |stream| {
                    drop(stream);
                    handler_returned_sender.send(()).unwrap();
                    completion
                },
            );
            finished_sender.send(result).unwrap();
        });

        handler_returned_receiver.recv().unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(
            matches!(finished_receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "final connection was released before its response owner completed"
        );
        response_owner.finish(ConnectionOutcome::Completed);
        thread::sleep(Duration::from_millis(100));
        assert!(
            matches!(finished_receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "final connection skipped its response delivery receipt"
        );
        client.shutdown(Shutdown::Both).unwrap();
        assert_eq!(
            finished_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Ok(())
        );
        join.join().unwrap();
    }

    #[test]
    fn completed_response_gets_an_independent_delivery_window() {
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
            "f".repeat(64),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let (handler_returned_sender, handler_returned_receiver) = mpsc::sync_channel(1);
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
        let revoker = generation.publication_revoker();
        let timing = BusinessConnectionTiming {
            hard_deadline: Duration::from_millis(300),
            delivery_receipt_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(5),
        };

        let join = thread::spawn(move || {
            let result = handle_business_with_timing(
                server,
                None,
                revoker,
                BusinessConnectionFinish::AwaitResponseDelivery,
                timing,
                |stream| {
                    thread::sleep(Duration::from_millis(50));
                    let completion = ConnectionCompletion::accepted();
                    completion.finish(ConnectionOutcome::Completed);
                    drop(stream);
                    handler_returned_sender.send(()).unwrap();
                    completion
                },
            );
            finished_sender.send(result).unwrap();
        });

        handler_returned_receiver.recv().unwrap();
        thread::sleep(Duration::from_millis(350));
        assert!(
            matches!(finished_receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "request watchdog consumed the response delivery window"
        );
        client.shutdown(Shutdown::Both).unwrap();
        assert_eq!(
            finished_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Ok(())
        );
        join.join().unwrap();
    }
}
