use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::supervisor::{
    DaemonBlockedReason, DaemonExitClass, DaemonLifecycleKind, DaemonLifecycleSnapshot,
    RestartLedgerReason,
};
use crate::daemon_response::DiagnosticsBody;
use crate::native_import::MAX_DIAGNOSTICS_EXPORT_BYTES;

const RECEIPT_SCHEMA: &str = "resume-ir.desktop-daemon-lifecycle-receipt.v1";
const DIAGNOSTICS_SCHEMA: &str = "resume-ir.desktop-diagnostics.v1";
const RECEIPT_FILE: &str = "desktop-daemon-lifecycle.v1.json";
const MAX_EVENTS: usize = 16;
const MAX_RECEIPT_BYTES: usize = 16 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const RECEIPT_QUEUE_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReceiptPersistenceState {
    Ready,
    RecoveredCorrupt,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LifecycleReceiptEvent {
    at_unix_ms: u64,
    state: DaemonLifecycleKind,
    generation: u64,
    restart_attempt: u8,
    restart_budget: u8,
    retry_delay_ms: Option<u64>,
    consecutive_heartbeat_failures: u8,
    blocked_reason: Option<DaemonBlockedReason>,
    last_exit: Option<DaemonExitClass>,
    #[serde(default)]
    restart_ledger_reason: Option<RestartLedgerReason>,
}

impl LifecycleReceiptEvent {
    fn capture(snapshot: &DaemonLifecycleSnapshot) -> Self {
        Self {
            at_unix_ms: unix_time_ms(),
            state: snapshot.state,
            generation: snapshot.generation.min(MAX_SAFE_INTEGER),
            restart_attempt: snapshot.restart_attempt,
            restart_budget: snapshot.restart_budget,
            retry_delay_ms: snapshot
                .retry_delay_ms
                .map(|value| value.min(MAX_SAFE_INTEGER)),
            consecutive_heartbeat_failures: snapshot.consecutive_heartbeat_failures,
            blocked_reason: snapshot.blocked_reason,
            last_exit: snapshot.last_exit,
            restart_ledger_reason: snapshot.restart_ledger_reason,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedLifecycleReceipt {
    schema_version: String,
    persistence_state: ReceiptPersistenceState,
    dropped_event_count: u64,
    events: VecDeque<LifecycleReceiptEvent>,
}

impl PersistedLifecycleReceipt {
    fn empty(persistence_state: ReceiptPersistenceState) -> Self {
        Self {
            schema_version: RECEIPT_SCHEMA.to_string(),
            persistence_state,
            dropped_event_count: 0,
            events: VecDeque::new(),
        }
    }

    fn append(&mut self, event: LifecycleReceiptEvent) {
        self.events.push_back(event);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
            self.record_drop();
        }
        while serialized_size(self).saturating_add(1) > MAX_RECEIPT_BYTES && self.events.len() > 1 {
            self.events.pop_front();
            self.record_drop();
        }
    }

    fn record_drop(&mut self) {
        self.dropped_event_count = self
            .dropped_event_count
            .saturating_add(1)
            .min(MAX_SAFE_INTEGER);
    }

    fn is_valid(&self) -> bool {
        self.schema_version == RECEIPT_SCHEMA
            && self.events.len() <= MAX_EVENTS
            && self.dropped_event_count <= MAX_SAFE_INTEGER
            && self
                .events
                .iter()
                .all(|event| event.at_unix_ms <= MAX_SAFE_INTEGER)
            && serialized_size(self).saturating_add(1) <= MAX_RECEIPT_BYTES
    }
}

#[derive(Clone, Debug, Serialize)]
struct LifecycleReceiptAggregate {
    schema_version: &'static str,
    persistence_state: ReceiptPersistenceState,
    dropped_event_count: u64,
    retained_event_count: usize,
    events: VecDeque<LifecycleReceiptEvent>,
}

impl From<PersistedLifecycleReceipt> for LifecycleReceiptAggregate {
    fn from(receipt: PersistedLifecycleReceipt) -> Self {
        Self {
            schema_version: RECEIPT_SCHEMA,
            persistence_state: receipt.persistence_state,
            dropped_event_count: receipt.dropped_event_count,
            retained_event_count: receipt.events.len(),
            events: receipt.events,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum DaemonDiagnosticsState {
    Included,
    Unavailable,
}

#[derive(Serialize)]
struct DesktopDiagnostics<'a> {
    schema_version: &'static str,
    privacy_boundary: &'static str,
    contains_raw_resume_text: bool,
    contains_queries: bool,
    contains_resume_paths: bool,
    contains_candidate_results: bool,
    contains_snippet_text: bool,
    lifecycle: LifecycleReceiptAggregate,
    daemon_diagnostics_state: DaemonDiagnosticsState,
    daemon_diagnostics: Option<&'a DiagnosticsBody>,
}

enum ReceiptCommand {
    Record(LifecycleReceiptEvent),
    Stop(mpsc::Sender<()>),
}

pub(super) struct LifecycleReceiptRecorder {
    data_dir: Option<PathBuf>,
    sender: Option<mpsc::SyncSender<ReceiptCommand>>,
    pending_dropped_events: Arc<AtomicU64>,
    cache: Arc<Mutex<PersistedLifecycleReceipt>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LifecycleReceiptRecorder {
    pub(super) fn initialize(data_dir: &Path) -> Self {
        let path = data_dir.join(RECEIPT_FILE);
        let receipt = load_receipt(&path);
        let cache = Arc::new(Mutex::new(receipt.clone()));
        let actor_cache = Arc::clone(&cache);
        let pending_dropped_events = Arc::new(AtomicU64::new(0));
        let actor_dropped_events = Arc::clone(&pending_dropped_events);
        let (sender, receiver) = mpsc::sync_channel(RECEIPT_QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name("resume-daemon-lifecycle-receipt".to_string())
            .spawn(move || {
                receipt_writer(path, receipt, actor_cache, actor_dropped_events, receiver);
            });
        match thread {
            Ok(thread) => Self {
                data_dir: Some(data_dir.to_path_buf()),
                sender: Some(sender),
                pending_dropped_events,
                cache,
                thread: Mutex::new(Some(thread)),
            },
            Err(_) => {
                if let Ok(mut receipt) = cache.lock() {
                    receipt.persistence_state = ReceiptPersistenceState::Unavailable;
                }
                Self {
                    data_dir: Some(data_dir.to_path_buf()),
                    sender: None,
                    pending_dropped_events,
                    cache,
                    thread: Mutex::new(None),
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            data_dir: None,
            sender: None,
            pending_dropped_events: Arc::new(AtomicU64::new(0)),
            cache: Arc::new(Mutex::new(PersistedLifecycleReceipt::empty(
                ReceiptPersistenceState::Unavailable,
            ))),
            thread: Mutex::new(None),
        }
    }

    pub(super) fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    pub(super) fn record(&self, snapshot: &DaemonLifecycleSnapshot) {
        if let Some(sender) = &self.sender {
            match sender.try_send(ReceiptCommand::Record(LifecycleReceiptEvent::capture(
                snapshot,
            ))) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    saturating_increment(&self.pending_dropped_events);
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    saturating_increment(&self.pending_dropped_events);
                    if let Ok(mut receipt) = self.cache.lock() {
                        receipt.persistence_state = ReceiptPersistenceState::Unavailable;
                    }
                }
            }
        }
    }

    pub(super) fn diagnostics(
        &self,
        daemon_diagnostics: Option<&DiagnosticsBody>,
    ) -> Result<Vec<u8>, crate::daemon_client::DesktopError> {
        let mut lifecycle = self
            .cache
            .lock()
            .map(|receipt| receipt.clone())
            .unwrap_or_else(|_| {
                PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Unavailable)
            });
        lifecycle.dropped_event_count = lifecycle
            .dropped_event_count
            .saturating_add(self.pending_dropped_events.load(Ordering::Acquire))
            .min(MAX_SAFE_INTEGER);
        let daemon_diagnostics_state = if daemon_diagnostics.is_some() {
            DaemonDiagnosticsState::Included
        } else {
            DaemonDiagnosticsState::Unavailable
        };
        let body = serde_json::to_vec_pretty(&DesktopDiagnostics {
            schema_version: DIAGNOSTICS_SCHEMA,
            privacy_boundary: "redacted_local_aggregate",
            contains_raw_resume_text: false,
            contains_queries: false,
            contains_resume_paths: false,
            contains_candidate_results: false,
            contains_snippet_text: false,
            lifecycle: lifecycle.into(),
            daemon_diagnostics_state,
            daemon_diagnostics,
        })
        .map_err(|_| {
            crate::daemon_client::DesktopError::new("diagnostics_invalid", "脱敏诊断无法序列化")
        })?;
        if body.len().saturating_add(1) > MAX_DIAGNOSTICS_EXPORT_BYTES {
            return Err(crate::daemon_client::DesktopError::new(
                "diagnostics_too_large",
                "脱敏诊断超过本地导出上限",
            ));
        }
        Ok(body)
    }

    pub(super) fn shutdown(&self) {
        let handle = self.thread.lock().ok().and_then(|mut slot| slot.take());
        let Some(handle) = handle else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let Some(commands) = &self.sender else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut command = ReceiptCommand::Stop(sender);
        loop {
            match commands.try_send(command) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    let _ = handle.join();
                    return;
                }
                Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                    command = returned;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::TrySendError::Full(_)) => {
                    // A stuck evidence-only writer owns no child process, runtime lock, or
                    // private payload. Dropping its handle keeps App shutdown bounded.
                    return;
                }
            }
        }
        if receiver.recv_timeout(Duration::from_secs(2)).is_ok() {
            let _ = handle.join();
        } else {
            // The same evidence-only boundary applies if fsync outlives the shutdown budget.
            // The OS will reap this in-process thread when the desktop process exits.
        }
    }
}

impl Drop for LifecycleReceiptRecorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn receipt_writer(
    path: PathBuf,
    mut receipt: PersistedLifecycleReceipt,
    cache: Arc<Mutex<PersistedLifecycleReceipt>>,
    pending_dropped_events: Arc<AtomicU64>,
    receiver: mpsc::Receiver<ReceiptCommand>,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            ReceiptCommand::Record(event) => {
                receipt.dropped_event_count = receipt
                    .dropped_event_count
                    .saturating_add(pending_dropped_events.swap(0, Ordering::AcqRel))
                    .min(MAX_SAFE_INTEGER);
                receipt.append(event);
                if receipt.persistence_state == ReceiptPersistenceState::Unavailable {
                    receipt.persistence_state = ReceiptPersistenceState::Ready;
                }
                if persist_receipt(&path, &receipt).is_err() {
                    receipt.persistence_state = ReceiptPersistenceState::Unavailable;
                }
                if let Ok(mut current) = cache.lock() {
                    *current = receipt.clone();
                }
            }
            ReceiptCommand::Stop(reply) => {
                let dropped = pending_dropped_events.swap(0, Ordering::AcqRel);
                if dropped > 0 {
                    receipt.dropped_event_count = receipt
                        .dropped_event_count
                        .saturating_add(dropped)
                        .min(MAX_SAFE_INTEGER);
                    if persist_receipt(&path, &receipt).is_err() {
                        receipt.persistence_state = ReceiptPersistenceState::Unavailable;
                    }
                    if let Ok(mut current) = cache.lock() {
                        *current = receipt.clone();
                    }
                }
                let _ = reply.send(());
                return;
            }
        }
    }
}

fn load_receipt(path: &Path) -> PersistedLifecycleReceipt {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Ready);
        }
        Err(_) => {
            return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Unavailable);
        }
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_RECEIPT_BYTES as u64 {
        return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::RecoveredCorrupt);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::RecoveredCorrupt);
    }
    let Ok(body) = fs::read(path) else {
        return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Unavailable);
    };
    let Ok(receipt) = serde_json::from_slice::<PersistedLifecycleReceipt>(&body) else {
        return PersistedLifecycleReceipt::empty(ReceiptPersistenceState::RecoveredCorrupt);
    };
    if receipt.is_valid() {
        receipt
    } else {
        PersistedLifecycleReceipt::empty(ReceiptPersistenceState::RecoveredCorrupt)
    }
}

fn persist_receipt(path: &Path, receipt: &PersistedLifecycleReceipt) -> std::io::Result<()> {
    if !receipt.is_valid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "lifecycle receipt exceeds its contract",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("receipt parent unavailable"))?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(temporary.as_file_mut(), receipt).map_err(std::io::Error::other)?;
    temporary.as_file_mut().write_all(b"\n")?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn serialized_size(receipt: &PersistedLifecycleReceipt) -> usize {
    serde_json::to_vec(receipt).map_or(usize::MAX, |body| body.len())
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(MAX_SAFE_INTEGER)
        .min(MAX_SAFE_INTEGER)
}

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_add(1).min(MAX_SAFE_INTEGER))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(generation: u64) -> DaemonLifecycleSnapshot {
        DaemonLifecycleSnapshot {
            schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
            state: DaemonLifecycleKind::Recovering,
            generation,
            restart_attempt: 2,
            restart_budget: 5,
            retry_delay_ms: Some(1_000),
            consecutive_heartbeat_failures: 0,
            blocked_reason: None,
            last_exit: Some(DaemonExitClass::ChildExited),
            restart_ledger_reason: None,
        }
    }

    #[test]
    fn receipt_retains_only_sixteen_owner_only_bounded_events() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(RECEIPT_FILE);
        let mut receipt = PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Ready);
        for generation in 0..40 {
            receipt.append(LifecycleReceiptEvent::capture(&snapshot(generation)));
        }
        persist_receipt(&path, &receipt).unwrap();

        let body = fs::read(&path).unwrap();
        let loaded = load_receipt(&path);
        assert_eq!(loaded.events.len(), MAX_EVENTS);
        assert_eq!(loaded.events.front().unwrap().generation, 24);
        assert_eq!(loaded.dropped_event_count, 24);
        assert!(body.len() <= MAX_RECEIPT_BYTES);
        #[cfg(unix)]
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }

    #[test]
    fn corrupt_receipt_is_discarded_without_blocking_new_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(RECEIPT_FILE);
        fs::write(&path, b"not-json").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let mut recovered = load_receipt(&path);
        assert_eq!(
            recovered.persistence_state,
            ReceiptPersistenceState::RecoveredCorrupt
        );
        assert!(recovered.events.is_empty());
        recovered.append(LifecycleReceiptEvent::capture(&snapshot(1)));
        persist_receipt(&path, &recovered).unwrap();
        assert_eq!(load_receipt(&path), recovered);
    }

    #[test]
    fn recorder_flushes_events_before_normal_shutdown_without_recording_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let recorder = LifecycleReceiptRecorder::initialize(directory.path());
        recorder.record(&snapshot(1));
        recorder.shutdown();

        let loaded = load_receipt(&directory.path().join(RECEIPT_FILE));
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].generation, 1);
        assert_eq!(loaded.events[0].state, DaemonLifecycleKind::Recovering);
    }

    #[test]
    fn receipt_contract_contains_no_process_or_private_payload_fields() {
        let mut receipt = PersistedLifecycleReceipt::empty(ReceiptPersistenceState::Ready);
        receipt.append(LifecycleReceiptEvent::capture(&snapshot(1)));
        let body = serde_json::to_string(&receipt).unwrap();
        for forbidden in ["pid", "path", "token", "query", "stderr", "resume_text"] {
            assert!(!body.contains(forbidden), "forbidden field: {forbidden}");
        }
    }

    #[test]
    fn desktop_diagnostics_remain_exportable_without_a_daemon() {
        let recorder = LifecycleReceiptRecorder::disabled();
        let body = recorder.diagnostics(None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["schema_version"], DIAGNOSTICS_SCHEMA);
        assert_eq!(value["daemon_diagnostics_state"], "unavailable");
        assert!(value["daemon_diagnostics"].is_null());
        assert_eq!(value["contains_resume_paths"], false);
        assert!(body.len() < MAX_DIAGNOSTICS_EXPORT_BYTES);
    }

    #[test]
    fn a_full_receipt_queue_never_blocks_the_supervisor_and_counts_the_drop() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let recorder = LifecycleReceiptRecorder {
            data_dir: None,
            sender: Some(sender),
            pending_dropped_events: Arc::new(AtomicU64::new(0)),
            cache: Arc::new(Mutex::new(PersistedLifecycleReceipt::empty(
                ReceiptPersistenceState::Ready,
            ))),
            thread: Mutex::new(None),
        };
        recorder.record(&snapshot(1));
        recorder.record(&snapshot(2));

        assert_eq!(recorder.pending_dropped_events.load(Ordering::Acquire), 1);
        let diagnostics: serde_json::Value =
            serde_json::from_slice(&recorder.diagnostics(None).unwrap()).unwrap();
        assert_eq!(diagnostics["lifecycle"]["dropped_event_count"], 1);
        drop(receiver);
    }
}
