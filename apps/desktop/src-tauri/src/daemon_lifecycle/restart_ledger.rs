use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::super::policy::{RestartPolicyConfig, MAX_AUTOMATIC_RESTARTS};

const LEDGER_SCHEMA: &str = "resume-ir.desktop-daemon-restart-window.v1";
const LEDGER_FILE: &str = "desktop-daemon-restart-window.v1.json";
const MAX_LEDGER_BYTES: u64 = 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::daemon_lifecycle) enum RestartLedgerReason {
    InvalidFormat,
    UnsafeFile,
    Oversized,
    ReadUnavailable,
    ClockInvalid,
    PersistenceUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedRestartWindow {
    schema_version: String,
    restart_attempts_unix_ms: VecDeque<u64>,
    scheduled_restart_not_before_unix_ms: Option<u64>,
    circuit_opened_at_unix_ms: Option<u64>,
    clean_shutdown_at_unix_ms: Option<u64>,
}

impl PersistedRestartWindow {
    fn empty() -> Self {
        Self {
            schema_version: LEDGER_SCHEMA.to_string(),
            restart_attempts_unix_ms: VecDeque::with_capacity(MAX_AUTOMATIC_RESTARTS),
            scheduled_restart_not_before_unix_ms: None,
            circuit_opened_at_unix_ms: None,
            clean_shutdown_at_unix_ms: None,
        }
    }

    fn is_valid(&self, now_unix_ms: u64, backoff: &[Duration; MAX_AUTOMATIC_RESTARTS]) -> bool {
        if self.schema_version != LEDGER_SCHEMA
            || self.restart_attempts_unix_ms.len() > MAX_AUTOMATIC_RESTARTS
            || self
                .restart_attempts_unix_ms
                .iter()
                .any(|at| *at > now_unix_ms || *at > MAX_SAFE_INTEGER)
            || !self
                .restart_attempts_unix_ms
                .iter()
                .zip(self.restart_attempts_unix_ms.iter().skip(1))
                .all(|(left, right)| left <= right)
        {
            return false;
        }
        let marker_count = usize::from(self.scheduled_restart_not_before_unix_ms.is_some())
            + usize::from(self.circuit_opened_at_unix_ms.is_some())
            + usize::from(self.clean_shutdown_at_unix_ms.is_some());
        if marker_count > 1 {
            return false;
        }
        if let Some(not_before) = self.scheduled_restart_not_before_unix_ms {
            let Some(index) = self.restart_attempts_unix_ms.len().checked_sub(1) else {
                return false;
            };
            let Some(last_attempt) = self.restart_attempts_unix_ms.back() else {
                return false;
            };
            if last_attempt.checked_add(duration_millis(backoff[index])) != Some(not_before)
                || not_before > MAX_SAFE_INTEGER
            {
                return false;
            }
        }
        if [
            self.circuit_opened_at_unix_ms,
            self.clean_shutdown_at_unix_ms,
        ]
        .into_iter()
        .flatten()
        .any(|at| {
            at > now_unix_ms
                || at > MAX_SAFE_INTEGER
                || self.restart_attempts_unix_ms.len() != MAX_AUTOMATIC_RESTARTS
                || self
                    .restart_attempts_unix_ms
                    .back()
                    .is_none_or(|last_attempt| *last_attempt > at)
        }) {
            return false;
        }
        self.restart_attempts_unix_ms.len() < MAX_AUTOMATIC_RESTARTS || marker_count == 1
    }
}

pub(super) struct RestartWindowLedger {
    path: Option<PathBuf>,
    config: RestartPolicyConfig,
    persisted: PersistedRestartWindow,
    reason: Option<RestartLedgerReason>,
}

impl RestartWindowLedger {
    pub(super) fn initialize(
        data_dir: &Path,
        now_unix_ms: u64,
        config: RestartPolicyConfig,
    ) -> Self {
        let path = data_dir.join(LEDGER_FILE);
        match load_ledger(&path, now_unix_ms, &config.backoff) {
            Ok(mut persisted) => {
                let changed = prune_expired(&mut persisted, now_unix_ms, config.window);
                let mut ledger = Self {
                    path: Some(path),
                    config,
                    persisted,
                    reason: None,
                };
                if changed && ledger.persist_current().is_err() {
                    ledger.reason = Some(RestartLedgerReason::PersistenceUnavailable);
                }
                ledger
            }
            Err(reason) => Self {
                path: Some(path),
                config,
                persisted: PersistedRestartWindow::empty(),
                reason: Some(reason),
            },
        }
    }

    pub(super) fn disabled(config: RestartPolicyConfig) -> Self {
        Self {
            path: None,
            config,
            persisted: PersistedRestartWindow::empty(),
            reason: None,
        }
    }

    pub(super) fn reason(&self) -> Option<RestartLedgerReason> {
        self.reason
    }

    pub(super) fn restart_attempt_ages(&self, now_unix_ms: u64) -> VecDeque<Duration> {
        self.persisted
            .restart_attempts_unix_ms
            .iter()
            .filter_map(|at| {
                let age = Duration::from_millis(now_unix_ms.saturating_sub(*at));
                (age < self.config.window).then_some(age)
            })
            .collect()
    }

    pub(super) fn circuit_remaining(
        &self,
        now_unix_ms: u64,
        circuit_open: Duration,
    ) -> Option<Duration> {
        self.persisted.circuit_opened_at_unix_ms.map(|opened_at| {
            circuit_open
                .saturating_sub(Duration::from_millis(now_unix_ms.saturating_sub(opened_at)))
        })
    }

    pub(super) fn scheduled_restart_remaining(&self, now_unix_ms: u64) -> Option<Duration> {
        self.persisted
            .scheduled_restart_not_before_unix_ms
            .map(|not_before| Duration::from_millis(not_before.saturating_sub(now_unix_ms)))
    }

    pub(super) fn record_restart_attempt(
        &mut self,
        now_unix_ms: u64,
        retry_delay: Duration,
    ) -> Result<(), RestartLedgerReason> {
        self.require_healthy_clock(now_unix_ms)?;
        let mut next = self.persisted.clone();
        prune_expired(&mut next, now_unix_ms, self.config.window);
        if next.restart_attempts_unix_ms.len() >= MAX_AUTOMATIC_RESTARTS {
            return self.fail(RestartLedgerReason::InvalidFormat);
        }
        if self.config.backoff[next.restart_attempts_unix_ms.len()] != retry_delay {
            return self.fail(RestartLedgerReason::InvalidFormat);
        }
        let Some(not_before) = now_unix_ms
            .checked_add(duration_millis(retry_delay))
            .filter(|at| *at <= MAX_SAFE_INTEGER)
        else {
            return self.fail(RestartLedgerReason::ClockInvalid);
        };
        next.restart_attempts_unix_ms.push_back(now_unix_ms);
        next.scheduled_restart_not_before_unix_ms = Some(not_before);
        self.persist_next(next)
    }

    pub(super) fn consume_start_authority(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<(), RestartLedgerReason> {
        self.require_healthy_clock(now_unix_ms)?;
        let mut next = self.persisted.clone();
        if let Some(not_before) = next.scheduled_restart_not_before_unix_ms {
            if now_unix_ms < not_before {
                return self.fail(RestartLedgerReason::ClockInvalid);
            }
            next.scheduled_restart_not_before_unix_ms = None;
        } else if next.clean_shutdown_at_unix_ms.is_some() {
            next.clean_shutdown_at_unix_ms = None;
        } else {
            return Ok(());
        }
        if next.restart_attempts_unix_ms.len() == MAX_AUTOMATIC_RESTARTS {
            next.circuit_opened_at_unix_ms = Some(now_unix_ms);
        }
        self.persist_next(next)
    }

    pub(super) fn record_circuit_open(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<(), RestartLedgerReason> {
        self.require_healthy_clock(now_unix_ms)?;
        if self.persisted.restart_attempts_unix_ms.len() != MAX_AUTOMATIC_RESTARTS {
            return self.fail(RestartLedgerReason::InvalidFormat);
        }
        let mut next = self.persisted.clone();
        next.scheduled_restart_not_before_unix_ms = None;
        next.clean_shutdown_at_unix_ms = None;
        next.circuit_opened_at_unix_ms = Some(now_unix_ms);
        self.persist_next(next)
    }

    pub(super) fn authorize_clean_restart(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<(), RestartLedgerReason> {
        self.require_healthy_clock(now_unix_ms)?;
        if self.persisted.restart_attempts_unix_ms.len() < MAX_AUTOMATIC_RESTARTS {
            return Ok(());
        }
        if self.persisted.circuit_opened_at_unix_ms.is_none() {
            return self.fail(RestartLedgerReason::InvalidFormat);
        }
        let mut next = self.persisted.clone();
        next.circuit_opened_at_unix_ms = None;
        next.clean_shutdown_at_unix_ms = Some(now_unix_ms);
        self.persist_next(next)
    }

    pub(super) fn record_probation_ready(
        &mut self,
        now_unix_ms: u64,
    ) -> Result<(), RestartLedgerReason> {
        self.require_healthy_clock(now_unix_ms)?;
        if self.persisted.circuit_opened_at_unix_ms.is_none() {
            return Ok(());
        }
        let mut next = self.persisted.clone();
        next.circuit_opened_at_unix_ms = Some(now_unix_ms);
        self.persist_next(next)
    }

    pub(super) fn clear_after_stable_ready(&mut self) -> Result<(), RestartLedgerReason> {
        if self.reason.is_some() {
            return Err(self
                .reason
                .unwrap_or(RestartLedgerReason::PersistenceUnavailable));
        }
        self.persist_next(PersistedRestartWindow::empty())
    }

    fn require_healthy_clock(&mut self, now_unix_ms: u64) -> Result<(), RestartLedgerReason> {
        if let Some(reason) = self.reason {
            return Err(reason);
        }
        let latest = self
            .persisted
            .circuit_opened_at_unix_ms
            .into_iter()
            .chain(self.persisted.clean_shutdown_at_unix_ms)
            .chain(self.persisted.restart_attempts_unix_ms.back().copied())
            .max();
        if latest.is_some_and(|at| at > now_unix_ms) {
            return self.fail(RestartLedgerReason::ClockInvalid);
        }
        Ok(())
    }

    fn persist_next(&mut self, next: PersistedRestartWindow) -> Result<(), RestartLedgerReason> {
        let Some(path) = &self.path else {
            self.persisted = next;
            self.reason = None;
            return Ok(());
        };
        if persist_ledger(path, &next).is_err() {
            return self.fail(RestartLedgerReason::PersistenceUnavailable);
        }
        self.persisted = next;
        self.reason = None;
        Ok(())
    }

    fn persist_current(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        persist_ledger(path, &self.persisted)
    }

    fn fail<T>(&mut self, reason: RestartLedgerReason) -> Result<T, RestartLedgerReason> {
        self.reason = Some(reason);
        Err(reason)
    }
}

pub(super) fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(MAX_SAFE_INTEGER)
        .min(MAX_SAFE_INTEGER)
}

fn load_ledger(
    path: &Path,
    now_unix_ms: u64,
    backoff: &[Duration; MAX_AUTOMATIC_RESTARTS],
) -> Result<PersistedRestartWindow, RestartLedgerReason> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedRestartWindow::empty());
        }
        Err(_) => return Err(RestartLedgerReason::ReadUnavailable),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RestartLedgerReason::UnsafeFile);
    }
    if metadata.len() > MAX_LEDGER_BYTES {
        return Err(RestartLedgerReason::Oversized);
    }
    if metadata.len() == 0 || !owner_only_permissions(&metadata) {
        return Err(if metadata.len() == 0 {
            RestartLedgerReason::InvalidFormat
        } else {
            RestartLedgerReason::UnsafeFile
        });
    }
    let file = File::open(path).map_err(|_| RestartLedgerReason::ReadUnavailable)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| RestartLedgerReason::ReadUnavailable)?;
    if !opened_metadata.is_file()
        || !owner_only_permissions(&opened_metadata)
        || !same_file_identity(&metadata, &opened_metadata)
    {
        return Err(RestartLedgerReason::UnsafeFile);
    }
    if opened_metadata.len() > MAX_LEDGER_BYTES {
        return Err(RestartLedgerReason::Oversized);
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.take(MAX_LEDGER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RestartLedgerReason::ReadUnavailable)?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err(RestartLedgerReason::Oversized);
    }
    let persisted: PersistedRestartWindow =
        serde_json::from_slice(&bytes).map_err(|_| RestartLedgerReason::InvalidFormat)?;
    if !persisted.is_valid(now_unix_ms, backoff) {
        return Err(
            if persisted
                .restart_attempts_unix_ms
                .iter()
                .chain(persisted.circuit_opened_at_unix_ms.iter())
                .chain(persisted.clean_shutdown_at_unix_ms.iter())
                .any(|at| *at > now_unix_ms)
            {
                RestartLedgerReason::ClockInvalid
            } else {
                RestartLedgerReason::InvalidFormat
            },
        );
    }
    Ok(persisted)
}

fn prune_expired(
    persisted: &mut PersistedRestartWindow,
    now_unix_ms: u64,
    window: Duration,
) -> bool {
    let original_len = persisted.restart_attempts_unix_ms.len();
    while persisted
        .restart_attempts_unix_ms
        .front()
        .is_some_and(|at| Duration::from_millis(now_unix_ms.saturating_sub(*at)) >= window)
    {
        persisted.restart_attempts_unix_ms.pop_front();
    }
    let attempts_changed = original_len != persisted.restart_attempts_unix_ms.len();
    if attempts_changed && persisted.restart_attempts_unix_ms.len() < MAX_AUTOMATIC_RESTARTS {
        persisted.scheduled_restart_not_before_unix_ms = None;
        persisted.circuit_opened_at_unix_ms = None;
        persisted.clean_shutdown_at_unix_ms = None;
    }
    attempts_changed
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn persist_ledger(path: &Path, persisted: &PersistedRestartWindow) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(persisted).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "restart ledger exceeds its bound",
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "restart ledger target is unsafe",
            ));
        }
        Ok(metadata) if !owner_only_permissions(&metadata) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "restart ledger permissions are unsafe",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("restart ledger parent unavailable"))?;
    fs::create_dir_all(parent)?;
    let temporary = write_temporary(parent, &bytes)?;
    let persisted_file = temporary.persist(path).map_err(|error| error.error)?;
    persisted_file.sync_all()?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn write_temporary(parent: &Path, bytes: &[u8]) -> std::io::Result<NamedTempFile> {
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(bytes)?;
    temporary.write_all(b"\n")?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

fn owner_only_permissions(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(unix)]
fn same_file_identity(expected: &fs::Metadata, opened: &fs::Metadata) -> bool {
    expected.dev() == opened.dev() && expected.ino() == opened.ino()
}

#[cfg(not(unix))]
fn same_file_identity(_expected: &fs::Metadata, _opened: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CONFIG: RestartPolicyConfig = RestartPolicyConfig {
        window: Duration::from_secs(60),
        stable_reset: Duration::from_secs(5),
        circuit_open: Duration::from_secs(5),
        backoff: [Duration::ZERO; MAX_AUTOMATIC_RESTARTS],
    };

    #[test]
    fn ledger_is_versioned_owner_only_bounded_and_private() {
        let directory = tempfile::tempdir().unwrap();
        let mut ledger = RestartWindowLedger::initialize(directory.path(), 10_000, TEST_CONFIG);
        for at in 10_000..10_005 {
            ledger.record_restart_attempt(at, Duration::ZERO).unwrap();
            ledger.consume_start_authority(at).unwrap();
        }
        ledger.record_circuit_open(10_005).unwrap();

        let path = directory.path().join(LEDGER_FILE);
        let body = fs::read(&path).unwrap();
        assert!(body.len() <= MAX_LEDGER_BYTES as usize);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["schema_version"], LEDGER_SCHEMA);
        assert_eq!(
            value["restart_attempts_unix_ms"].as_array().unwrap().len(),
            MAX_AUTOMATIC_RESTARTS
        );
        #[cfg(unix)]
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
        let encoded = String::from_utf8(body).unwrap();
        for forbidden in ["pid", "path", "token", "stderr", "query", "resume_text"] {
            assert!(!encoded.contains(forbidden), "forbidden field: {forbidden}");
        }
    }

    #[test]
    fn reopen_prunes_attempts_outside_the_wall_clock_window() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LEDGER_FILE);
        let persisted = PersistedRestartWindow {
            schema_version: LEDGER_SCHEMA.to_string(),
            restart_attempts_unix_ms: VecDeque::from([40_000, 40_001, 100_000]),
            scheduled_restart_not_before_unix_ms: None,
            circuit_opened_at_unix_ms: None,
            clean_shutdown_at_unix_ms: None,
        };
        persist_ledger(&path, &persisted).unwrap();

        let reopened = RestartWindowLedger::initialize(directory.path(), 100_000, TEST_CONFIG);
        assert_eq!(
            reopened.restart_attempt_ages(100_000),
            VecDeque::from([Duration::from_millis(59_999), Duration::ZERO])
        );
        let body: PersistedRestartWindow =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            body.restart_attempts_unix_ms,
            VecDeque::from([40_001, 100_000])
        );
    }

    #[test]
    fn scheduled_deadline_must_match_the_current_fixed_backoff() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LEDGER_FILE);
        let mut persisted = PersistedRestartWindow::empty();
        persisted.restart_attempts_unix_ms.push_back(10_000);
        persisted.scheduled_restart_not_before_unix_ms = Some(10_000 + 10 * 60 * 1_000);
        persist_ledger(&path, &persisted).unwrap();

        assert_eq!(
            RestartWindowLedger::initialize(
                directory.path(),
                10_000,
                RestartPolicyConfig::production(),
            )
            .reason(),
            Some(RestartLedgerReason::InvalidFormat)
        );

        persisted.scheduled_restart_not_before_unix_ms = Some(10_250);
        persist_ledger(&path, &persisted).unwrap();
        let valid = RestartWindowLedger::initialize(
            directory.path(),
            10_000,
            RestartPolicyConfig::production(),
        );
        assert_eq!(valid.reason(), None);
        assert_eq!(
            valid.scheduled_restart_remaining(10_000),
            Some(Duration::from_millis(250))
        );
    }

    #[test]
    fn corrupt_oversized_and_unsafe_ledgers_return_bounded_reasons() {
        let corrupt = tempfile::tempdir().unwrap();
        let corrupt_path = corrupt.path().join(LEDGER_FILE);
        fs::write(&corrupt_path, b"not-json").unwrap();
        set_owner_only(&corrupt_path);
        assert_eq!(
            RestartWindowLedger::initialize(corrupt.path(), 10_000, TEST_CONFIG).reason(),
            Some(RestartLedgerReason::InvalidFormat)
        );
        assert_eq!(fs::read(corrupt_path).unwrap(), b"not-json");

        let unbounded_schedule = tempfile::tempdir().unwrap();
        let unbounded_schedule_path = unbounded_schedule.path().join(LEDGER_FILE);
        let mut body = PersistedRestartWindow::empty();
        body.restart_attempts_unix_ms.push_back(10_000);
        body.scheduled_restart_not_before_unix_ms = Some(MAX_SAFE_INTEGER);
        persist_ledger(&unbounded_schedule_path, &body).unwrap();
        assert_eq!(
            RestartWindowLedger::initialize(
                unbounded_schedule.path(),
                10_000,
                RestartPolicyConfig::production(),
            )
            .reason(),
            Some(RestartLedgerReason::InvalidFormat)
        );

        let oversized = tempfile::tempdir().unwrap();
        let oversized_path = oversized.path().join(LEDGER_FILE);
        fs::write(&oversized_path, vec![b'x'; MAX_LEDGER_BYTES as usize + 1]).unwrap();
        set_owner_only(&oversized_path);
        assert_eq!(
            RestartWindowLedger::initialize(oversized.path(), 10_000, TEST_CONFIG).reason(),
            Some(RestartLedgerReason::Oversized)
        );

        #[cfg(unix)]
        {
            let unsafe_directory = tempfile::tempdir().unwrap();
            let unsafe_path = unsafe_directory.path().join(LEDGER_FILE);
            fs::write(&unsafe_path, b"{}").unwrap();
            fs::set_permissions(&unsafe_path, fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                RestartWindowLedger::initialize(unsafe_directory.path(), 10_000, TEST_CONFIG)
                    .reason(),
                Some(RestartLedgerReason::UnsafeFile)
            );
        }
    }

    #[test]
    fn temporary_write_failure_cannot_replace_the_committed_ledger() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LEDGER_FILE);
        let committed = PersistedRestartWindow::empty();
        persist_ledger(&path, &committed).unwrap();
        let before = fs::read(&path).unwrap();

        let temporary = write_temporary(directory.path(), b"synthetic-uncommitted").unwrap();
        drop(temporary);

        assert_eq!(fs::read(path).unwrap(), before);
    }

    fn set_owner_only(path: &Path) {
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
