use std::fmt;
use std::path::Path;
use std::rc::Rc;

use crate::data_directory_owner::SearchPublicationOwnershipGuard;
use crate::{ContentDigest, OwnedMetaStore, Result};

/// Unforgeable ownership for one search-publication critical section.
///
/// A session can only be acquired from an [`OwnedMetaStore`]. It retains both
/// canonical data-directory ownership and the shared `search-publication.lock`
/// for its complete lifetime. Migration rebuild attempt reservation and
/// completion are available only through this capability.
///
/// The session is deliberately thread-affine. Moving a live holder to another
/// thread would make thread-identity recursion detection unsound.
///
/// ```compile_fail
/// # use meta_store::SearchPublicationSession;
/// fn require_send<T: Send>() {}
/// require_send::<SearchPublicationSession>();
/// ```
#[must_use = "dropping the session releases exclusive search-publication ownership"]
pub struct SearchPublicationSession {
    store: OwnedMetaStore,
    ownership: Rc<SearchPublicationOwnershipGuard>,
    active_attempt_id: Option<ContentDigest>,
}

/// Retained search-publication ownership for fallible artifact hand-offs.
///
/// This lease is derived from a [`SearchPublicationSession`] and cannot be
/// acquired from a path or synthesized by downstream crates.
///
/// ```compile_fail
/// # use meta_store::SearchPublicationLease;
/// fn require_send<T: Send>() {}
/// require_send::<SearchPublicationLease>();
/// ```
#[must_use = "dropping the lease may release exclusive search-publication ownership"]
pub struct SearchPublicationLease {
    ownership: Rc<SearchPublicationOwnershipGuard>,
}

impl OwnedMetaStore {
    /// Waits in the process-local FIFO for the single search-publication
    /// namespace bound to this store's canonical owner generation.
    ///
    /// A recursive call on the holding thread is a runtime invariant failure.
    /// A conflicting process-external lock is never waited on and returns a
    /// typed ownership error.
    pub fn wait_for_search_publication_session(&self) -> Result<SearchPublicationSession> {
        self.access
            .guard()
            .ensure_search_publication_wait_is_not_reentrant()?;
        // Open the dedicated metadata connection before taking the publication
        // lock. Store preparation may itself need that lock during a migration
        // cutover, so reversing this order could self-deadlock.
        let store = self.open_sibling()?;
        let ownership = Rc::new(
            store
                .access
                .guard()
                .wait_for_search_publication_ownership()?,
        );
        Ok(SearchPublicationSession {
            store,
            ownership,
            active_attempt_id: None,
        })
    }

    /// Tries to acquire the single search-publication namespace without
    /// waiting or issuing a FIFO ticket when another writer is active.
    ///
    /// Same-thread recursion is a typed storage invariant failure. Another
    /// process-local holder or waiter, and an external OS-lock holder, return
    /// a typed ownership error so maintenance callers can defer safely.
    pub fn try_acquire_search_publication_session(&self) -> Result<SearchPublicationSession> {
        self.access
            .guard()
            .ensure_search_publication_try_is_uncontended()?;
        let store = self.open_sibling()?;
        let ownership = Rc::new(
            store
                .access
                .guard()
                .try_acquire_search_publication_ownership()?,
        );
        Ok(SearchPublicationSession {
            store,
            ownership,
            active_attempt_id: None,
        })
    }
}

impl SearchPublicationSession {
    /// Retains the same OS-lock generation across a fallible artifact hand-off.
    pub fn retain(&self) -> SearchPublicationLease {
        SearchPublicationLease {
            ownership: Rc::clone(&self.ownership),
        }
    }

    /// Returns the exact owner-bound metadata store carried by this session.
    /// The reference cannot outlive publication ownership.
    pub fn owned_store(&self) -> &OwnedMetaStore {
        &self.store
    }

    /// Returns the exact canonical data directory bound to this session's
    /// retained owner generation.
    pub fn canonical_data_dir(&self) -> &Path {
        self.store.access.guard().canonical_data_dir()
    }

    pub(crate) fn active_attempt_id(&self) -> Option<&ContentDigest> {
        self.active_attempt_id.as_ref()
    }

    pub(crate) fn set_active_attempt_id(&mut self, attempt_id: ContentDigest) {
        self.active_attempt_id = Some(attempt_id);
    }

    pub(crate) fn clear_active_attempt_if(&mut self, attempt_id: &ContentDigest) {
        if self.active_attempt_id.as_ref() == Some(attempt_id) {
            self.active_attempt_id = None;
        }
    }
}

impl SearchPublicationLease {
    /// Retains the same publication generation for another fallible owner.
    pub fn retain(&self) -> Self {
        Self {
            ownership: Rc::clone(&self.ownership),
        }
    }
}

impl fmt::Debug for SearchPublicationSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchPublicationSession(<redacted>)")
    }
}

impl fmt::Debug for SearchPublicationLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchPublicationLease(<redacted>)")
    }
}
