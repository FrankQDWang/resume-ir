use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use rusqlite::Connection;

use crate::data_directory_owner::DataDirectoryOwnerGuard;
use crate::MetadataEncryptionState;

mod sealed {
    pub trait Access {}
    pub trait WriteAccess: Access {}
}

/// Sealed metadata-store access mode used to share read APIs without making
/// write authority forgeable outside this crate.
#[doc(hidden)]
pub trait MetadataStoreAccess: sealed::Access {}

/// Sealed access mode implemented only by owner-bound and in-memory stores.
#[doc(hidden)]
pub trait MetadataStoreWriteAccess: MetadataStoreAccess + sealed::WriteAccess {}

/// Generic implementation detail behind the three concrete public store
/// types. The containing module is private, so callers cannot name an access
/// mode or construct a capability themselves.
pub struct MetadataStore<Access: MetadataStoreAccess> {
    pub(crate) connection: RefCell<Connection>,
    pub(crate) metadata_encryption_state: MetadataEncryptionState,
    pub(crate) file_backed: bool,
    pub(crate) access: Access,
}

#[doc(hidden)]
pub struct ReadStoreAccess {
    _private: (),
}

impl ReadStoreAccess {
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for ReadStoreAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadStoreAccess")
    }
}

#[doc(hidden)]
pub struct OwnedStoreAccess {
    guard: Arc<DataDirectoryOwnerGuard>,
}

impl OwnedStoreAccess {
    pub(crate) fn new(guard: Arc<DataDirectoryOwnerGuard>) -> Self {
        Self { guard }
    }

    pub(crate) fn guard(&self) -> &Arc<DataDirectoryOwnerGuard> {
        &self.guard
    }
}

impl fmt::Debug for OwnedStoreAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OwnedStoreAccess(<redacted>)")
    }
}

#[doc(hidden)]
pub struct EphemeralStoreAccess {
    _private: (),
}

impl EphemeralStoreAccess {
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for EphemeralStoreAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EphemeralStoreAccess")
    }
}

impl sealed::Access for ReadStoreAccess {}
impl sealed::Access for OwnedStoreAccess {}
impl sealed::Access for EphemeralStoreAccess {}
impl sealed::WriteAccess for OwnedStoreAccess {}
impl sealed::WriteAccess for EphemeralStoreAccess {}

impl MetadataStoreAccess for ReadStoreAccess {}
impl MetadataStoreAccess for OwnedStoreAccess {}
impl MetadataStoreAccess for EphemeralStoreAccess {}
impl MetadataStoreWriteAccess for OwnedStoreAccess {}
impl MetadataStoreWriteAccess for EphemeralStoreAccess {}
