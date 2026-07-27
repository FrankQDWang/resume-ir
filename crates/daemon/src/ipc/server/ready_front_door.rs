use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ipc::connection::{self, PreparedBusinessConnection, ReadyAdmission};
use crate::ipc::{ControlPlaneState, DaemonFatalError};

use super::connection_lifecycle::LISTENER_POLL_INTERVAL;
use super::{classify_accept_error, AcceptErrorDisposition};

const BUSINESS_QUEUE_CAPACITY: usize = 16;
const MAX_ACTIVE_ADMISSIONS: usize = 8;
const ADMISSION_HARD_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadyFrontDoorStop {
    Requested,
    ParentShutdown,
    RequestLimitReached,
}

/// Owns the ready listener and serves the in-memory control plane independently
/// from the serialized metadata-writing lane.
pub(super) struct ReadyFrontDoor {
    stop: Arc<AtomicBool>,
    business_receiver: Receiver<PreparedBusinessConnection>,
    join: JoinHandle<Result<ReadyFrontDoorStop, DaemonFatalError>>,
}

impl ReadyFrontDoor {
    pub(super) fn start(
        listener: TcpListener,
        state: ControlPlaneState,
        auth_token: String,
        shutdown: Option<Arc<AtomicBool>>,
        request_limit: Option<usize>,
    ) -> Result<Self, DaemonFatalError> {
        let (business_sender, business_receiver) = mpsc::sync_channel(BUSINESS_QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("resume-ipc-front-door".to_string())
            .spawn(move || {
                run(
                    listener,
                    state,
                    auth_token,
                    business_sender,
                    thread_stop,
                    shutdown,
                    request_limit.unwrap_or(usize::MAX),
                )
            })
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
        Ok(Self {
            stop,
            business_receiver,
            join,
        })
    }

    pub(super) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<PreparedBusinessConnection, RecvTimeoutError> {
        self.business_receiver.recv_timeout(timeout)
    }

    pub(super) fn join(self) -> Result<ReadyFrontDoorStop, DaemonFatalError> {
        self.join
            .join()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?
    }

    pub(super) fn stop_and_join(self) -> Result<ReadyFrontDoorStop, DaemonFatalError> {
        self.stop.store(true, Ordering::Release);
        self.join()
    }
}

struct ActiveAdmission<'scope> {
    cancellation: TcpStream,
    join: thread::ScopedJoinHandle<'scope, ReadyAdmission>,
    deadline: Instant,
}

impl ActiveAdmission<'_> {
    fn cancel(&self) {
        let _ = self.cancellation.shutdown(Shutdown::Both);
    }

    fn cancel_if_expired(&self) {
        if Instant::now() >= self.deadline {
            self.cancel();
        }
    }

    fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    fn join(self) -> Result<(), DaemonFatalError> {
        let outcome = self
            .join
            .join()
            .map_err(|_| DaemonFatalError::ControlPlaneFailure)?;
        if outcome == ReadyAdmission::Completed {
            let _ = self.cancellation.shutdown(Shutdown::Write);
        }
        Ok(())
    }
}

fn run(
    listener: TcpListener,
    state: ControlPlaneState,
    auth_token: String,
    business_sender: mpsc::SyncSender<PreparedBusinessConnection>,
    stop: Arc<AtomicBool>,
    shutdown: Option<Arc<AtomicBool>>,
    request_limit: usize,
) -> Result<ReadyFrontDoorStop, DaemonFatalError> {
    thread::scope(|scope| {
        let mut active = Vec::new();
        let mut accepted = 0_usize;
        loop {
            if stop.load(Ordering::Acquire) {
                cancel_and_join_all(&mut active)?;
                return Ok(ReadyFrontDoorStop::Requested);
            }
            if shutdown
                .as_ref()
                .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
            {
                cancel_and_join_all(&mut active)?;
                return Ok(ReadyFrontDoorStop::ParentShutdown);
            }
            reap_finished(&mut active)?;
            if accepted >= request_limit {
                if active.is_empty() {
                    return Ok(ReadyFrontDoorStop::RequestLimitReached);
                }
                thread::sleep(LISTENER_POLL_INTERVAL);
                continue;
            }
            if active.len() >= MAX_ACTIVE_ADMISSIONS {
                for admission in &active {
                    admission.cancel_if_expired();
                }
                thread::sleep(LISTENER_POLL_INTERVAL);
                continue;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    accepted = accepted.saturating_add(1);
                    let final_request = accepted == request_limit;
                    let cancellation = match stream.try_clone() {
                        Ok(cancellation) => cancellation,
                        Err(_) => continue,
                    };
                    let state = &state;
                    let token = auth_token.as_str();
                    let sender = &business_sender;
                    active.push(ActiveAdmission {
                        cancellation,
                        join: scope.spawn(move || {
                            connection::admit_ready(stream, state, token, sender, final_request)
                        }),
                        deadline: Instant::now()
                            .checked_add(ADMISSION_HARD_DEADLINE)
                            .unwrap_or_else(Instant::now),
                    });
                }
                Err(error) => match classify_accept_error(error.kind()) {
                    AcceptErrorDisposition::NoConnectionReady
                    | AcceptErrorDisposition::ConnectionLocal => {
                        thread::sleep(LISTENER_POLL_INTERVAL);
                    }
                    AcceptErrorDisposition::ListenerFatal => {
                        cancel_and_join_all(&mut active)?;
                        return Err(DaemonFatalError::ControlPlaneFailure);
                    }
                },
            }
        }
    })
}

fn reap_finished(active: &mut Vec<ActiveAdmission<'_>>) -> Result<(), DaemonFatalError> {
    let mut index = 0;
    while index < active.len() {
        if active[index].is_finished() {
            active.swap_remove(index).join()?;
        } else {
            active[index].cancel_if_expired();
            index += 1;
        }
    }
    Ok(())
}

fn cancel_and_join_all(active: &mut Vec<ActiveAdmission<'_>>) -> Result<(), DaemonFatalError> {
    for admission in active.iter() {
        admission.cancel();
    }
    while let Some(admission) = active.pop() {
        admission.join()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    use super::{ReadyFrontDoor, ReadyFrontDoorStop};
    use crate::ipc::{ControlPlaneState, CoreHealth, OptionalRuntimeHealth, OptionalRuntimeMatrix};

    #[test]
    fn authenticated_status_bypasses_pending_business_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let token = "a".repeat(64);
        let (state, mut publisher) = ControlPlaneState::initializing();
        let available = OptionalRuntimeHealth::available();
        publisher
            .publish_without_store_for_test(
                CoreHealth::ready(),
                OptionalRuntimeMatrix {
                    embedding: available,
                    ocr: available,
                    classifier: available,
                    pdfium: available,
                },
            )
            .unwrap();
        let front_door =
            ReadyFrontDoor::start(listener, state, token.clone(), None, Some(2)).unwrap();

        let mut business_client = TcpStream::connect(address).unwrap();
        write_request(&mut business_client, "POST", "/imports", &token);
        let pending = front_door
            .recv_timeout(Duration::from_secs(1))
            .expect("business request reaches the serialized data lane");

        let mut status_client = TcpStream::connect(address).unwrap();
        status_client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write_request(&mut status_client, "GET", "/status", &token);
        let mut response = String::new();
        status_client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#""schema_version":"daemon.status.v5""#));

        drop(pending);
        assert_eq!(
            front_door.join().unwrap(),
            ReadyFrontDoorStop::RequestLimitReached
        );
    }

    fn write_request(stream: &mut TcpStream, method: &str, path: &str, token: &str) {
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    }
}
