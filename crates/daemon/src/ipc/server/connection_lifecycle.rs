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
const FINAL_RESPONSE_PEER_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
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
    let watchdog = thread::spawn(move || {
        let deadline = Instant::now()
            .checked_add(CONNECTION_HARD_DEADLINE)
            .unwrap_or_else(Instant::now);
        let mut deadline_cancelled = false;
        loop {
            if watcher_finished.load(Ordering::Acquire) {
                return;
            }
            if shutdown
                .as_ref()
                .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
            {
                publication_revoker.withdraw();
                let _ = cancellation.shutdown(Shutdown::Both);
                return;
            }
            if !deadline_cancelled && Instant::now() >= deadline {
                let _ = cancellation.shutdown(Shutdown::Both);
                deadline_cancelled = true;
            }
            thread::sleep(LISTENER_POLL_INTERVAL);
        }
    });

    handle(stream);
    if let Some(mut peer_close) = peer_close {
        let _ = peer_close.set_read_timeout(Some(FINAL_RESPONSE_PEER_CLOSE_TIMEOUT));
        let _ = peer_close.read(&mut [0_u8; 1]);
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
