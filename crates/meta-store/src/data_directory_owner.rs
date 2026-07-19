use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, ThreadId};

use crate::{
    ImportTaskStatus, MetaStoreError, OwnedMetaStore, Result as StoreResult, UnixTimestamp,
};

mod lock_ops;
mod task_lock;

use lock_ops::{ExclusiveLockAttempt, LockOpenErrorClass};

pub(crate) use task_lock::acquire_legacy_task_locks;
pub use task_lock::{import_task_owner_lock_path, ImportTaskOwnerLock};

const DATA_DIRECTORY_OWNER_LOCK_FILE: &str = "data-directory-owner.lock";
const LEGACY_DAEMON_OWNER_LOCK_FILE: &str = "daemon.owner.lock";
const SEARCH_PUBLICATION_LOCK_FILE: &str = "search-publication.lock";
#[cfg(windows)]
const FILE_SHARE_READ: u32 = 0x0000_0001;
#[cfg(windows)]
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDirectoryOwnerAcquireError {
    Storage,
    RuntimeIntegrity,
}

impl fmt::Display for DataDirectoryOwnerAcquireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage => formatter.write_str("data-directory owner storage unavailable"),
            Self::RuntimeIntegrity => {
                formatter.write_str("data-directory owner lock integrity failed")
            }
        }
    }
}

impl std::error::Error for DataDirectoryOwnerAcquireError {}

#[derive(Debug)]
pub enum ImportProcessingOrphanNormalizationError {
    Store(MetaStoreError),
    TaskOwnerLockStorage,
    TaskOwnerLockContended,
}

impl fmt::Display for ImportProcessingOrphanNormalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::TaskOwnerLockStorage => formatter.write_str("import task owner lock unavailable"),
            Self::TaskOwnerLockContended => {
                formatter.write_str("import task owner conflicts with data-directory owner")
            }
        }
    }
}

impl std::error::Error for ImportProcessingOrphanNormalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::TaskOwnerLockStorage | Self::TaskOwnerLockContended => None,
        }
    }
}

pub enum DataDirectoryOwnerAcquisition {
    Acquired(DataDirectoryOwnerLease),
    Contended,
}

impl fmt::Debug for DataDirectoryOwnerAcquisition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acquired(_) => formatter.write_str("Acquired(<lease>)"),
            Self::Contended => formatter.write_str("Contended"),
        }
    }
}

/// Exclusive, canonical data-directory capability for metadata publication and
/// import-processing lifecycle mutations.
///
/// The capability is intentionally non-cloneable and cannot be constructed by
/// callers. A conforming daemon or offline writer must retain one lease for its
/// complete generation. Metadata creation and copy-on-write migration accept
/// this capability instead of a caller-supplied path.
#[must_use = "dropping the lease releases data-directory storage/import ownership"]
pub struct DataDirectoryOwnerLease {
    guard: Arc<DataDirectoryOwnerGuard>,
}

/// Shared process-generation guard retained by all metadata write capabilities.
///
/// The public lease is deliberately non-cloneable. Internal owned-store APIs
/// may clone this guard so the kernel locks cannot be released while any writer
/// derived from the lease remains live.
pub(crate) struct DataDirectoryOwnerGuard {
    data_directory_lock: File,
    legacy_daemon_lock: File,
    data_dir: PathBuf,
    search_publication_arbiter: Arc<SearchPublicationArbiter>,
}

impl fmt::Debug for DataDirectoryOwnerLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DataDirectoryOwnerLease(<redacted>)")
    }
}

impl DataDirectoryOwnerLease {
    /// Attempts to become the only storage/import owner for one canonical data
    /// directory without waiting or retrying.
    pub fn try_acquire(
        data_dir: &Path,
    ) -> std::result::Result<DataDirectoryOwnerAcquisition, DataDirectoryOwnerAcquireError> {
        fs::create_dir_all(data_dir).map_err(|_| DataDirectoryOwnerAcquireError::Storage)?;
        let data_dir =
            fs::canonicalize(data_dir).map_err(|_| DataDirectoryOwnerAcquireError::Storage)?;
        let path = data_dir.join(DATA_DIRECTORY_OWNER_LOCK_FILE);
        let data_directory_lock = open_owner_lock_file(&path)?;
        match lock_ops::try_exclusive(&data_directory_lock) {
            Ok(ExclusiveLockAttempt::Acquired) => {
                validate_open_owner_lock_file(&path, &data_directory_lock)?;
                let daemon_path = data_dir.join(LEGACY_DAEMON_OWNER_LOCK_FILE);
                let legacy_daemon_lock = open_owner_lock_file(&daemon_path)?;
                match lock_ops::try_exclusive(&legacy_daemon_lock) {
                    Ok(ExclusiveLockAttempt::Acquired) => {
                        validate_open_owner_lock_file(&daemon_path, &legacy_daemon_lock)?;
                    }
                    Ok(ExclusiveLockAttempt::Contended) => {
                        return Ok(DataDirectoryOwnerAcquisition::Contended);
                    }
                    Err(_) => {
                        return Err(DataDirectoryOwnerAcquireError::Storage);
                    }
                }
                Ok(DataDirectoryOwnerAcquisition::Acquired(Self {
                    guard: Arc::new(DataDirectoryOwnerGuard {
                        data_directory_lock,
                        legacy_daemon_lock,
                        data_dir,
                        search_publication_arbiter: Arc::new(SearchPublicationArbiter::new()),
                    }),
                }))
            }
            Ok(ExclusiveLockAttempt::Contended) => Ok(DataDirectoryOwnerAcquisition::Contended),
            Err(_) => Err(DataDirectoryOwnerAcquireError::Storage),
        }
    }

    /// Opens the current v28 store, creating or copy-on-write migrating it when
    /// necessary. The bound canonical directory cannot be substituted by the
    /// caller.
    pub fn open_store(&self) -> StoreResult<OwnedMetaStore> {
        OwnedMetaStore::open_data_dir_for_owner(self)
    }

    pub fn canonical_data_dir(&self) -> &Path {
        self.guard.canonical_data_dir()
    }

    pub(crate) fn shared_guard(&self) -> Arc<DataDirectoryOwnerGuard> {
        Arc::clone(&self.guard)
    }
}

impl OwnedMetaStore {
    /// Normalizes attempts left running by a previous process generation.
    ///
    /// This store's retained owner guard proves that no conforming import
    /// processor can race the normalization. Per-task locks still fail closed
    /// against an older binary outside the global ownership protocol.
    pub fn normalize_orphaned_running_tasks(
        &self,
        now: UnixTimestamp,
    ) -> std::result::Result<usize, ImportProcessingOrphanNormalizationError> {
        let observed_tasks = self
            .running_import_tasks_for_owner_normalization()
            .map_err(ImportProcessingOrphanNormalizationError::Store)?;
        let mut normalized = 0_usize;
        for observed in observed_tasks {
            let owner = ImportTaskOwnerLock::try_acquire(
                self.access.guard().canonical_data_dir(),
                &observed.id,
            )
            .map_err(|_| ImportProcessingOrphanNormalizationError::TaskOwnerLockStorage)?
            .ok_or(ImportProcessingOrphanNormalizationError::TaskOwnerLockContended)?;
            let Some(current) = self
                .import_task_by_id(&observed.id)
                .map_err(ImportProcessingOrphanNormalizationError::Store)?
            else {
                drop(owner);
                continue;
            };
            if current.status != ImportTaskStatus::Running {
                drop(owner);
                continue;
            }
            if self
                .normalize_observed_orphaned_running_import_task(&current, now)
                .map_err(ImportProcessingOrphanNormalizationError::Store)?
            {
                normalized += 1;
            }
            drop(owner);
        }
        Ok(normalized)
    }
}

impl DataDirectoryOwnerGuard {
    pub(crate) fn canonical_data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Rejects recursive blocking acquisition before store preparation.
    /// Store preparation deliberately precedes the blocking arbiter because it
    /// may need to publish a migration cutover itself.
    pub(crate) fn ensure_search_publication_wait_is_not_reentrant(&self) -> StoreResult<()> {
        self.search_publication_arbiter.ensure_not_reentrant()
    }

    /// Rejects a known in-process contender before nonblocking store
    /// preparation. The arbiter is checked again when the permit is acquired,
    /// so a contender arriving after this preflight still fails closed.
    pub(crate) fn ensure_search_publication_try_is_uncontended(&self) -> StoreResult<()> {
        self.search_publication_arbiter
            .ensure_fail_fast_uncontended()
    }

    /// Waits in the process-local FIFO for the search-publication namespace.
    /// The process-external OS lock remains nonblocking and fails closed.
    pub(crate) fn wait_for_search_publication_ownership(
        self: &Arc<Self>,
    ) -> StoreResult<SearchPublicationOwnershipGuard> {
        let permit = self.search_publication_arbiter.wait_acquire()?;
        self.acquire_search_publication_os_lock(permit)
    }

    /// Attempts to acquire the search-publication namespace without issuing a
    /// FIFO ticket when any in-process writer is already holding or waiting.
    pub(crate) fn try_acquire_search_publication_ownership(
        self: &Arc<Self>,
    ) -> StoreResult<SearchPublicationOwnershipGuard> {
        let permit = self.search_publication_arbiter.try_acquire()?;
        self.acquire_search_publication_os_lock(permit)
    }

    fn acquire_search_publication_os_lock(
        self: &Arc<Self>,
        permit: SearchPublicationPermit,
    ) -> StoreResult<SearchPublicationOwnershipGuard> {
        let path = self.data_dir.join(SEARCH_PUBLICATION_LOCK_FILE);
        let file = open_legacy_publication_lock_file(&path)?;
        match lock_ops::try_exclusive(&file) {
            Ok(ExclusiveLockAttempt::Acquired) => {
                validate_open_legacy_publication_lock_file(&path, &file)?;
                Ok(SearchPublicationOwnershipGuard {
                    file,
                    _permit: permit,
                    _owner: Arc::clone(self),
                })
            }
            Ok(ExclusiveLockAttempt::Contended) => {
                Err(MetaStoreError::migration_ownership_required())
            }
            Err(error) => Err(MetaStoreError::io_storage(error)),
        }
    }

    #[cfg(test)]
    fn wait_for_search_publication_waiters(&self, expected: usize) -> StoreResult<()> {
        self.search_publication_arbiter
            .wait_for_waiters_for_test(expected)
    }
}

/// Fair generation-local serialization in front of the process-external OS
/// lock. Tickets define one total order for all writer connections derived from
/// the same [`DataDirectoryOwnerGuard`].
struct SearchPublicationArbiter {
    state: Mutex<SearchPublicationArbiterState>,
    changed: Condvar,
}

#[derive(Default)]
struct SearchPublicationArbiterState {
    next_ticket: u64,
    serving_ticket: u64,
    holder: Option<ThreadId>,
    integrity_failed: bool,
}

impl SearchPublicationArbiter {
    fn new() -> Self {
        Self {
            state: Mutex::new(SearchPublicationArbiterState::default()),
            changed: Condvar::new(),
        }
    }

    fn ensure_not_reentrant(&self) -> StoreResult<()> {
        let state = self.lock_healthy_state()?;
        if state.holder.as_ref() == Some(&thread::current().id()) {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(())
    }

    fn ensure_fail_fast_uncontended(&self) -> StoreResult<()> {
        let state = self.lock_healthy_state()?;
        if state.holder.as_ref() == Some(&thread::current().id()) {
            return Err(MetaStoreError::storage_invariant());
        }
        if state.holder.is_some() || state.next_ticket != state.serving_ticket {
            return Err(MetaStoreError::migration_ownership_required());
        }
        Ok(())
    }

    fn wait_acquire(self: &Arc<Self>) -> StoreResult<SearchPublicationPermit> {
        let thread_id = thread::current().id();
        let mut state = self.lock_healthy_state()?;
        if state.holder.as_ref() == Some(&thread_id) {
            return Err(MetaStoreError::storage_invariant());
        }
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(MetaStoreError::storage_invariant)?;
        self.changed.notify_all();

        while state.holder.is_some() || state.serving_ticket != ticket {
            state = self.wait_for_healthy_state(state)?;
        }
        state.holder = Some(thread_id);
        Ok(SearchPublicationPermit {
            arbiter: Arc::clone(self),
            ticket,
            thread_id,
            _thread_affinity: PhantomData,
        })
    }

    fn try_acquire(self: &Arc<Self>) -> StoreResult<SearchPublicationPermit> {
        let thread_id = thread::current().id();
        let mut state = self.lock_healthy_state()?;
        if state.holder.as_ref() == Some(&thread_id) {
            return Err(MetaStoreError::storage_invariant());
        }
        if state.holder.is_some() || state.next_ticket != state.serving_ticket {
            return Err(MetaStoreError::migration_ownership_required());
        }
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(MetaStoreError::storage_invariant)?;
        state.holder = Some(thread_id);
        Ok(SearchPublicationPermit {
            arbiter: Arc::clone(self),
            ticket,
            thread_id,
            _thread_affinity: PhantomData,
        })
    }

    fn release(&self, ticket: u64, thread_id: ThreadId) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.integrity_failed = true;
                state
            }
        };
        if state.holder.as_ref() != Some(&thread_id) || state.serving_ticket != ticket {
            state.integrity_failed = true;
            state.holder = None;
            self.changed.notify_all();
            return;
        }
        state.holder = None;
        match state.serving_ticket.checked_add(1) {
            Some(next) => state.serving_ticket = next,
            None => state.integrity_failed = true,
        }
        self.changed.notify_all();
    }

    fn lock_healthy_state(&self) -> StoreResult<MutexGuard<'_, SearchPublicationArbiterState>> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.integrity_failed = true;
                self.changed.notify_all();
                return Err(MetaStoreError::storage_invariant());
            }
        };
        if state.integrity_failed {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(state)
    }

    fn wait_for_healthy_state<'a>(
        &self,
        state: MutexGuard<'a, SearchPublicationArbiterState>,
    ) -> StoreResult<MutexGuard<'a, SearchPublicationArbiterState>> {
        let state = match self.changed.wait(state) {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.integrity_failed = true;
                self.changed.notify_all();
                return Err(MetaStoreError::storage_invariant());
            }
        };
        if state.integrity_failed {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(state)
    }

    #[cfg(test)]
    fn wait_for_waiters_for_test(&self, expected: usize) -> StoreResult<()> {
        let mut state = self.lock_healthy_state()?;
        while waiting_count(&state)? < expected {
            state = self.wait_for_healthy_state(state)?;
        }
        Ok(())
    }
}

struct SearchPublicationPermit {
    arbiter: Arc<SearchPublicationArbiter>,
    ticket: u64,
    thread_id: ThreadId,
    _thread_affinity: PhantomData<Rc<()>>,
}

impl Drop for SearchPublicationPermit {
    fn drop(&mut self) {
        self.arbiter.release(self.ticket, self.thread_id);
    }
}

#[cfg(test)]
fn waiting_count(state: &SearchPublicationArbiterState) -> StoreResult<usize> {
    let outstanding = state
        .next_ticket
        .checked_sub(state.serving_ticket)
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let waiting = outstanding
        .checked_sub(u64::from(state.holder.is_some()))
        .ok_or_else(MetaStoreError::storage_invariant)?;
    usize::try_from(waiting).map_err(|_| MetaStoreError::storage_invariant())
}

impl Drop for DataDirectoryOwnerGuard {
    fn drop(&mut self) {
        let _ = lock_ops::unlock(&self.legacy_daemon_lock);
        let _ = lock_ops::unlock(&self.data_directory_lock);
    }
}

/// Composite guard for canonical data-directory and search-publication
/// ownership. Retaining the owner `Arc` prevents either kernel lock from being
/// released while publication work remains live.
pub(crate) struct SearchPublicationOwnershipGuard {
    file: File,
    _permit: SearchPublicationPermit,
    _owner: Arc<DataDirectoryOwnerGuard>,
}

impl Drop for SearchPublicationOwnershipGuard {
    fn drop(&mut self) {
        let _ = lock_ops::unlock(&self.file);
    }
}

fn open_owner_lock_file(path: &Path) -> std::result::Result<File, DataDirectoryOwnerAcquireError> {
    validate_existing_owner_lock_path(path)?;
    #[cfg(windows)]
    let mut options = private_lock_options();
    #[cfg(not(windows))]
    let options = private_lock_options();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let file = options
        .open(path)
        .map_err(|_| DataDirectoryOwnerAcquireError::Storage)?;
    validate_open_owner_lock_file(path, &file)?;
    Ok(file)
}

fn open_legacy_publication_lock_file(path: &Path) -> StoreResult<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_legacy_publication_lock_metadata(&metadata)?,
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    #[cfg(windows)]
    let mut options = private_lock_options();
    #[cfg(not(windows))]
    let options = private_lock_options();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let file = options.open(path).map_err(|error| {
        match lock_ops::classify_current_open_error(&error) {
            LockOpenErrorClass::Contended => MetaStoreError::migration_ownership_required(),
            LockOpenErrorClass::Storage => MetaStoreError::io_storage(error),
        }
    })?;
    validate_open_legacy_publication_lock_file(path, &file)?;
    Ok(file)
}

fn validate_open_legacy_publication_lock_file(path: &Path, file: &File) -> StoreResult<()> {
    let opened = file.metadata().map_err(MetaStoreError::io_storage)?;
    validate_legacy_publication_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
    validate_legacy_publication_lock_metadata(&current)?;
    if !same_file_identity(file, path, &opened, &current).map_err(MetaStoreError::io_storage)? {
        return Err(MetaStoreError::invalid_value(
            "metadata.legacy_publication_lock",
        ));
    }
    Ok(())
}

fn validate_legacy_publication_lock_metadata(metadata: &fs::Metadata) -> StoreResult<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(MetaStoreError::invalid_value(
            "metadata.legacy_publication_lock",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(MetaStoreError::invalid_value(
                "metadata.legacy_publication_lock",
            ));
        }
    }
    Ok(())
}

fn validate_existing_owner_lock_path(
    path: &Path,
) -> std::result::Result<(), DataDirectoryOwnerAcquireError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_owner_lock_metadata(&metadata),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DataDirectoryOwnerAcquireError::Storage),
    }
}

fn validate_open_owner_lock_file(
    path: &Path,
    file: &File,
) -> std::result::Result<(), DataDirectoryOwnerAcquireError> {
    let opened = file
        .metadata()
        .map_err(|_| DataDirectoryOwnerAcquireError::Storage)?;
    validate_owner_lock_metadata(&opened)?;
    let current =
        fs::symlink_metadata(path).map_err(|_| DataDirectoryOwnerAcquireError::Storage)?;
    validate_owner_lock_metadata(&current)?;
    if !same_file_identity(file, path, &opened, &current)
        .map_err(|_| DataDirectoryOwnerAcquireError::Storage)?
    {
        return Err(DataDirectoryOwnerAcquireError::RuntimeIntegrity);
    }
    Ok(())
}

fn validate_owner_lock_metadata(
    metadata: &fs::Metadata,
) -> std::result::Result<(), DataDirectoryOwnerAcquireError> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(DataDirectoryOwnerAcquireError::RuntimeIntegrity);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(DataDirectoryOwnerAcquireError::RuntimeIntegrity);
        }
    }
    Ok(())
}

fn private_lock_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

#[cfg(unix)]
fn same_file_identity(
    _file: &File,
    _path: &Path,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(opened.dev() == current.dev() && opened.ino() == current.ino())
}

#[cfg(windows)]
fn same_file_identity(
    file: &File,
    path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> io::Result<bool> {
    let opened = same_file::Handle::from_file(file.try_clone()?)?;
    let current = same_file::Handle::from_path(path)?;
    let final_metadata = fs::symlink_metadata(path)?;
    if !final_metadata.file_type().is_file() || final_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(opened == current)
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(
    _file: &File,
    _path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
#[path = "data_directory_owner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "data_directory_owner/lock_ops_tests.rs"]
mod lock_ops_tests;
