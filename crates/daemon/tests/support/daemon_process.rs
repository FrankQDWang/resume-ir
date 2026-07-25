use std::io::{self, BufRead, BufReader, Read};
use std::process::{ChildStderr, ChildStdout, ExitStatus};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use process_containment::ContainedChild;

const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Drains daemon stdout for the lifetime of a test child and publishes the
/// first control-plane endpoint without making a blocking pipe read the owner
/// of the caller's timeout.
pub struct DaemonStdout {
    endpoint: Receiver<io::Result<String>>,
    join: Option<JoinHandle<()>>,
}

pub fn spawn_daemon_stdout(stdout: ChildStdout) -> DaemonStdout {
    let (endpoint_sender, endpoint_receiver) = mpsc::sync_channel(1);
    let join = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut endpoint_sent = false;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    if !endpoint_sent {
                        let _ = endpoint_sender.send(Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "daemon stdout closed before endpoint publication",
                        )));
                    }
                    return;
                }
                Ok(_) => {
                    let Some(endpoint) = line.trim().strip_prefix("ipc status endpoint: ") else {
                        continue;
                    };
                    if !endpoint_sent {
                        endpoint_sent = true;
                        let _ = endpoint_sender.send(Ok(endpoint.to_string()));
                    }
                }
                Err(error) => {
                    if !endpoint_sent {
                        let _ = endpoint_sender.send(Err(error));
                    }
                    return;
                }
            }
        }
    });
    DaemonStdout {
        endpoint: endpoint_receiver,
        join: Some(join),
    }
}

impl DaemonStdout {
    pub fn wait_for_endpoint(
        &mut self,
        child: &mut ContainedChild,
        stderr: &mut ChildStderr,
        timeout: Duration,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                child.terminate();
                self.finish();
                panic!("daemon did not publish its control-plane endpoint before timeout");
            }
            match self.endpoint.recv_timeout(remaining.min(POLL_INTERVAL)) {
                Ok(Ok(endpoint)) => return endpoint,
                Ok(Err(error)) => {
                    child.terminate();
                    self.finish();
                    panic!(
                        "daemon stdout failed before endpoint publication: {:?}",
                        error.kind()
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    child.terminate();
                    self.finish();
                    panic!("daemon stdout collector stopped before endpoint publication");
                }
            }
            if let Some(status) = child.try_wait().expect("poll daemon before endpoint") {
                self.finish();
                let mut stderr_body = String::new();
                let _ = stderr.read_to_string(&mut stderr_body);
                panic!(
                    "daemon exited before endpoint publication: {status}; stderr_empty={}",
                    stderr_body.is_empty()
                );
            }
        }
    }

    pub fn finish(&mut self) {
        self.join
            .take()
            .expect("daemon stdout collector is joined once")
            .join()
            .expect("join daemon stdout collector");
    }
}

pub fn wait_for_exit(
    child: &mut ContainedChild,
    timeout: Duration,
    context: &'static str,
) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll contained daemon child") {
            return status;
        }
        if Instant::now() >= deadline {
            child.terminate();
            panic!("daemon did not exit before timeout: {context}");
        }
        thread::sleep(POLL_INTERVAL);
    }
}
