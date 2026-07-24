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
pub(super) const LISTENER_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ControlLoopStop {
    Handoff,
    ParentShutdown,
    RequestLimitReached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BusinessConnectionFinish {
    Immediate,
    AwaitPeerClose,
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
    handle: impl FnOnce(TcpStream),
) -> Result<(), DaemonFatalError> {
    let cancellation = match stream.try_clone() {
        Ok(cancellation) => cancellation,
        Err(_) => return Ok(()),
    };
    let peer_close = matches!(finish, BusinessConnectionFinish::AwaitPeerClose)
        .then(|| stream.try_clone().ok())
        .flatten();
    let finished = Arc::new(AtomicBool::new(false));
    let watcher_finished = Arc::clone(&finished);
    let cancelled = Arc::new(AtomicBool::new(false));
    let watcher_cancelled = Arc::clone(&cancelled);
    let watchdog = thread::spawn(move || {
        let deadline = Instant::now()
            .checked_add(CONNECTION_HARD_DEADLINE)
            .unwrap_or_else(Instant::now);
        loop {
            if watcher_finished.load(Ordering::Acquire) {
                return;
            }
            if shutdown
                .as_ref()
                .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
            {
                publication_revoker.withdraw();
                watcher_cancelled.store(true, Ordering::Release);
                let _ = cancellation.shutdown(Shutdown::Both);
                return;
            }
            if Instant::now() >= deadline {
                watcher_cancelled.store(true, Ordering::Release);
                let _ = cancellation.shutdown(Shutdown::Both);
                return;
            }
            thread::sleep(LISTENER_POLL_INTERVAL);
        }
    });

    handle(stream);
    if let Some(mut peer_close) = peer_close {
        let _ = peer_close.set_read_timeout(Some(LISTENER_POLL_INTERVAL));
        while !cancelled.load(Ordering::Acquire) {
            match peer_close.read(&mut [0_u8; 1]) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => break,
            }
        }
    }
    finished.store(true, Ordering::Release);
    watchdog
        .join()
        .map_err(|_| DaemonFatalError::ControlPlaneFailure)
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

    use super::{handle_business_with_watchdog, BusinessConnectionFinish};
    use crate::ipc::generation::{DaemonGenerationOwner, OwnerMode};

    #[test]
    fn final_connection_ignores_poll_timeouts_until_peer_close_or_watchdog() {
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

        let join = thread::spawn(move || {
            let result = handle_business_with_watchdog(
                server,
                None,
                revoker,
                BusinessConnectionFinish::AwaitPeerClose,
                |stream| {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(25)))
                        .unwrap();
                    drop(stream);
                    handler_returned_sender.send(()).unwrap();
                },
            );
            finished_sender.send(result).unwrap();
        });

        handler_returned_receiver.recv().unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(
            matches!(finished_receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "final connection was released before its peer closed"
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
