use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use process_containment::ContainedChild;

use super::supervisor::{
    ChildExitOutcome, DaemonBlockedReason, DaemonProbe, DaemonRuntime, RuntimeFailure,
    SupervisedChild,
};
use super::{
    classifier::configured_classifier_runtime, configured_daemon_binary,
    configured_embedding_runtime, configured_ocr_runtime, daemon_arguments,
};
use crate::daemon_client;

const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(25);
const FATAL_EVENT_MAX_BYTES: usize = 1024;
const FATAL_EVENT_RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);

/// Proof local to this module that the contained child can no longer hold stderr open.
struct ChildTerminated;

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum DaemonFatalClass {
    OwnershipConflict,
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    ControlPlaneFailure,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum FatalDisposition {
    Blocked,
    Restartable,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonFatalEvent {
    schema_version: String,
    event: String,
    class: DaemonFatalClass,
    disposition: FatalDisposition,
}

impl DaemonFatalEvent {
    fn outcome(self) -> Option<ChildExitOutcome> {
        if self.schema_version != "resume-ir.daemon-fatal.v1" || self.event != "fatal" {
            return None;
        }
        match (self.class, self.disposition) {
            (DaemonFatalClass::OwnershipConflict, FatalDisposition::Blocked) => Some(
                ChildExitOutcome::Blocked(DaemonBlockedReason::OwnershipConflict),
            ),
            (DaemonFatalClass::ConfigurationInvalid, FatalDisposition::Blocked) => Some(
                ChildExitOutcome::Blocked(DaemonBlockedReason::ConfigurationInvalid),
            ),
            (DaemonFatalClass::RuntimeIntegrity, FatalDisposition::Blocked) => Some(
                ChildExitOutcome::Blocked(DaemonBlockedReason::RuntimeIntegrity),
            ),
            (DaemonFatalClass::ProtocolMismatch, FatalDisposition::Blocked) => Some(
                ChildExitOutcome::Blocked(DaemonBlockedReason::ProtocolMismatch),
            ),
            (DaemonFatalClass::ControlPlaneFailure, FatalDisposition::Restartable) => {
                Some(ChildExitOutcome::RestartableFatal)
            }
            _ => None,
        }
    }
}

pub(super) struct ProductionDaemonRuntime {
    data_dir: PathBuf,
    current_exe: PathBuf,
    embedding_resource_dir: PathBuf,
    ocr_resource_dir: PathBuf,
    classifier_resource_dir: PathBuf,
}

impl ProductionDaemonRuntime {
    pub(super) fn initialize(
        data_dir: &Path,
        current_exe: &Path,
        embedding_resource_dir: &Path,
        ocr_resource_dir: &Path,
        classifier_resource_dir: &Path,
    ) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            current_exe: current_exe.to_path_buf(),
            embedding_resource_dir: embedding_resource_dir.to_path_buf(),
            ocr_resource_dir: ocr_resource_dir.to_path_buf(),
            classifier_resource_dir: classifier_resource_dir.to_path_buf(),
        }
    }
}

impl DaemonRuntime for ProductionDaemonRuntime {
    type Child = OwnedDaemon;

    fn spawn(&mut self) -> Result<Self::Child, RuntimeFailure> {
        let binary = configured_daemon_binary()
            .map_err(|_| RuntimeFailure::Blocked(DaemonBlockedReason::RuntimeIntegrity))?;
        let embedding =
            configured_embedding_runtime(&self.current_exe, &self.embedding_resource_dir)
                .map_err(|_| RuntimeFailure::Blocked(DaemonBlockedReason::ConfigurationInvalid))?;
        let ocr = configured_ocr_runtime(&self.current_exe, &self.ocr_resource_dir)
            .map_err(|_| RuntimeFailure::Blocked(DaemonBlockedReason::ConfigurationInvalid))?;
        let classifier = configured_classifier_runtime(&self.classifier_resource_dir)
            .map_err(|_| RuntimeFailure::Blocked(DaemonBlockedReason::ConfigurationInvalid))?;
        let mut command = Command::new(binary);
        command
            .args(daemon_arguments(
                &self.data_dir,
                embedding.as_ref(),
                ocr.as_ref(),
                classifier.as_ref(),
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(embedding) = &embedding {
            embedding
                .configure_command(&mut command)
                .map_err(|_| RuntimeFailure::Blocked(DaemonBlockedReason::ConfigurationInvalid))?;
        }
        if let Some(ocr) = &ocr {
            ocr.configure_command(&mut command);
        }
        let mut process =
            ContainedChild::spawn(&mut command).map_err(|_| RuntimeFailure::Transient)?;
        let Some(lifecycle_stdin) = process.take_stdin() else {
            process.terminate();
            return Err(RuntimeFailure::Transient);
        };
        let Some(stderr) = process.take_stderr() else {
            process.terminate();
            return Err(RuntimeFailure::Transient);
        };
        let fatal_reader = FatalEventReader::spawn(stderr);
        Ok(OwnedDaemon {
            process,
            lifecycle_stdin: Some(lifecycle_stdin),
            fatal_reader,
        })
    }

    fn probe(&mut self, timeout: Duration) -> DaemonProbe {
        match daemon_client::execute_status_probe_from_with_timeout(&self.data_dir, timeout) {
            Ok(_) => DaemonProbe::Ready,
            Err(error) if error.is_daemon_unavailable() => DaemonProbe::Unavailable,
            Err(_) => DaemonProbe::ProtocolMismatch,
        }
    }
}

pub(super) struct OwnedDaemon {
    process: ContainedChild,
    lifecycle_stdin: Option<ChildStdin>,
    fatal_reader: FatalEventReader,
}

impl OwnedDaemon {
    fn stop(mut self) {
        drop(self.lifecycle_stdin.take());
        let deadline = Instant::now() + SHUTDOWN_GRACE_PERIOD;
        while Instant::now() < deadline {
            if self.process.try_wait().ok().flatten().is_some() {
                self.fatal_reader.finish(ChildTerminated);
                return;
            }
            thread::sleep(SHUTDOWN_POLL_INTERVAL);
        }
        self.process.terminate();
        self.fatal_reader.finish(ChildTerminated);
    }
}

impl SupervisedChild for OwnedDaemon {
    fn poll_exit(&mut self) -> ChildExitOutcome {
        match self.process.try_wait() {
            Ok(Some(_)) => self
                .fatal_reader
                .take_outcome_after_exit(ChildTerminated)
                .unwrap_or(ChildExitOutcome::Exited),
            Err(_) => ChildExitOutcome::Exited,
            Ok(None) => ChildExitOutcome::Running,
        }
    }

    fn stop(self) {
        OwnedDaemon::stop(self);
    }
}

impl Drop for OwnedDaemon {
    fn drop(&mut self) {
        drop(self.lifecycle_stdin.take());
        self.process.terminate();
        self.fatal_reader.finish(ChildTerminated);
    }
}

struct FatalEventReader {
    receiver: mpsc::Receiver<Option<ChildExitOutcome>>,
    thread: Option<JoinHandle<()>>,
}

impl FatalEventReader {
    fn spawn(stderr: ChildStderr) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("resume-daemon-fatal-reader".to_string())
            .spawn(move || {
                let _ = sender.send(read_fatal_event(stderr));
            })
            .ok();
        Self { receiver, thread }
    }

    fn take_outcome_after_exit(&mut self, terminated: ChildTerminated) -> Option<ChildExitOutcome> {
        let outcome = self
            .receiver
            .recv_timeout(FATAL_EVENT_RECEIVE_TIMEOUT)
            .ok()
            .flatten();
        self.finish(terminated);
        outcome
    }

    fn finish(&mut self, _terminated: ChildTerminated) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_fatal_event(mut stderr: impl Read) -> Option<ChildExitOutcome> {
    let mut first_line = Vec::with_capacity(256);
    let mut buffer = [0_u8; 256];
    while let Ok(read) = stderr.read(&mut buffer) {
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            if *byte == b'\n' {
                return decode_fatal_event(&mut first_line);
            }
            if first_line.len() < FATAL_EVENT_MAX_BYTES {
                first_line.push(*byte);
            } else {
                return None;
            }
        }
    }
    decode_fatal_event(&mut first_line)
}

fn decode_fatal_event(first_line: &mut Vec<u8>) -> Option<ChildExitOutcome> {
    if first_line.last() == Some(&b'\r') {
        first_line.pop();
    }
    serde_json::from_slice::<DaemonFatalEvent>(first_line)
        .ok()
        .and_then(DaemonFatalEvent::outcome)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn accepts_only_the_closed_bounded_fatal_contract() {
        let blocked = b"{\"schema_version\":\"resume-ir.daemon-fatal.v1\",\"event\":\"fatal\",\"class\":\"ownership_conflict\",\"disposition\":\"blocked\"}\nignored raw stderr";
        assert_eq!(
            read_fatal_event(Cursor::new(blocked)),
            Some(ChildExitOutcome::Blocked(
                DaemonBlockedReason::OwnershipConflict
            ))
        );

        let restartable = b"{\"schema_version\":\"resume-ir.daemon-fatal.v1\",\"event\":\"fatal\",\"class\":\"control_plane_failure\",\"disposition\":\"restartable\"}\n";
        assert_eq!(
            read_fatal_event(Cursor::new(restartable)),
            Some(ChildExitOutcome::RestartableFatal)
        );
    }

    #[test]
    fn rejects_unknown_fields_mismatched_dispositions_and_oversized_lines() {
        let extra = b"{\"schema_version\":\"resume-ir.daemon-fatal.v1\",\"event\":\"fatal\",\"class\":\"runtime_integrity\",\"disposition\":\"blocked\",\"message\":\"private\"}\n";
        assert_eq!(read_fatal_event(Cursor::new(extra)), None);

        let mismatch = b"{\"schema_version\":\"resume-ir.daemon-fatal.v1\",\"event\":\"fatal\",\"class\":\"control_plane_failure\",\"disposition\":\"blocked\"}\n";
        assert_eq!(read_fatal_event(Cursor::new(mismatch)), None);

        let oversized = vec![b'x'; FATAL_EVENT_MAX_BYTES + 1];
        assert_eq!(read_fatal_event(Cursor::new(oversized)), None);
    }
}
