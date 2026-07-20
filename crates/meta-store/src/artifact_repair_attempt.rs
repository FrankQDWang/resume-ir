use std::fmt;

use crate::{
    ContentDigest, MetadataStore, MetadataStoreAccess, Result, SearchPublicationSession,
    UnixTimestamp,
};

#[path = "artifact_repair_attempt_persistence.rs"]
mod persistence;
#[path = "artifact_repair_attempt_transaction.rs"]
mod transaction;

pub const ARTIFACT_REPAIR_MAX_ATTEMPTS: u8 = 5;
pub(super) const MAX_ATTEMPTS: u8 = ARTIFACT_REPAIR_MAX_ATTEMPTS;
pub(super) const RETRY_DELAYS_SECONDS: [i64; MAX_ATTEMPTS as usize] = [1, 4, 15, 30, 60];

#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactRepairKey {
    pub(super) generation: String,
    pub(super) publication_fingerprint: ContentDigest,
    pub(super) visible_epoch: u64,
}

impl ArtifactRepairKey {
    pub fn new(
        generation: String,
        publication_fingerprint: ContentDigest,
        visible_epoch: u64,
    ) -> Self {
        Self {
            generation,
            publication_fingerprint,
            visible_epoch,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn publication_fingerprint(&self) -> &ContentDigest {
        &self.publication_fingerprint
    }

    pub fn visible_epoch(&self) -> u64 {
        self.visible_epoch
    }
}

impl fmt::Debug for ArtifactRepairKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactRepairKey")
            .field("generation", &"<redacted>")
            .field("publication_fingerprint", &"<redacted>")
            .field("visible_epoch", &self.visible_epoch)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptErrorKind {
    FullTextPublicationBusy,
    FullTextFailure,
    VectorPublicationBusy,
    VectorFailure,
    MetadataFailure,
    Cleanup,
    Interrupted,
}

impl ArtifactRepairAttemptErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullTextPublicationBusy => "fulltext_publication_busy",
            Self::FullTextFailure => "fulltext_failure",
            Self::VectorPublicationBusy => "vector_publication_busy",
            Self::VectorFailure => "vector_failure",
            Self::MetadataFailure => "metadata_failure",
            Self::Cleanup => "cleanup",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptFailure {
    Retryable(ArtifactRepairAttemptErrorKind),
    Terminal(ArtifactRepairAttemptErrorKind),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactRepairAttempt {
    pub(super) key: ArtifactRepairKey,
    pub(super) attempt_id: ContentDigest,
    pub(super) attempt_count: u8,
    pub(super) prior_retry: Option<ArtifactRepairRetrySnapshot>,
}

impl fmt::Debug for ArtifactRepairAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactRepairAttempt")
            .field("key", &self.key)
            .field("attempt_id", &"<redacted>")
            .field("attempt_count", &self.attempt_count)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptAcquire {
    Started(ArtifactRepairAttempt),
    InProgress,
    NotDue,
    RepairBlocked,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptFailureOutcome {
    RetryScheduled,
    RepairBlocked,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptCancellationOutcome {
    Restored,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRepairAttemptPhase {
    Running,
    RetryWait,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRepairAttemptState {
    pub attempt_count: u8,
    pub phase: ArtifactRepairAttemptPhase,
    pub started_at: UnixTimestamp,
    pub next_retry_at: Option<UnixTimestamp>,
    pub last_error_kind: Option<ArtifactRepairAttemptErrorKind>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ArtifactRepairRetrySnapshot {
    pub(super) attempt_count: u8,
    pub(super) started_at: UnixTimestamp,
    pub(super) next_retry_at: UnixTimestamp,
    pub(super) last_error_kind: ArtifactRepairAttemptErrorKind,
    pub(super) updated_at: UnixTimestamp,
}

pub(super) struct ArtifactRepairAttemptRecord {
    pub(super) generation: String,
    pub(super) publication_fingerprint: ContentDigest,
    pub(super) visible_epoch: u64,
    pub(super) attempt_id: ContentDigest,
    pub(super) attempt_count: u8,
    pub(super) phase: ArtifactRepairAttemptPhase,
    pub(super) started_at: UnixTimestamp,
    pub(super) next_retry_at: Option<UnixTimestamp>,
    pub(super) last_error_kind: Option<ArtifactRepairAttemptErrorKind>,
    pub(super) updated_at: UnixTimestamp,
}

impl SearchPublicationSession {
    /// Reserves one durable attempt only while the exact artifact repair
    /// context remains authoritative. Fast ticks and process restarts cannot
    /// bypass the persisted deadline or five-attempt budget.
    pub fn acquire_artifact_repair_attempt(
        &mut self,
        key: &ArtifactRepairKey,
        now: UnixTimestamp,
    ) -> Result<ArtifactRepairAttemptAcquire> {
        let outcome =
            transaction::acquire_attempt(self.owned_store(), self.active_attempt_id(), key, now)?;
        if let ArtifactRepairAttemptAcquire::Started(attempt) = &outcome {
            self.set_active_attempt_id(attempt.attempt_id.clone());
        }
        Ok(outcome)
    }

    pub fn finish_artifact_repair_attempt_failure(
        &mut self,
        attempt: &ArtifactRepairAttempt,
        failure: ArtifactRepairAttemptFailure,
        now: UnixTimestamp,
    ) -> Result<ArtifactRepairAttemptFailureOutcome> {
        let outcome =
            transaction::finish_attempt_failure(self.owned_store(), attempt, failure, now)?;
        self.clear_active_attempt_if(&attempt.attempt_id);
        Ok(outcome)
    }

    /// Restores the retry state that existed before this lifecycle-cancelled
    /// reservation. A shutdown or explicit cancellation never consumes an
    /// artifact repair attempt.
    pub fn cancel_artifact_repair_attempt(
        &mut self,
        attempt: &ArtifactRepairAttempt,
    ) -> Result<ArtifactRepairAttemptCancellationOutcome> {
        let outcome = transaction::cancel_attempt(self.owned_store(), attempt)?;
        self.clear_active_attempt_if(&attempt.attempt_id);
        Ok(outcome)
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn artifact_repair_attempt_state(&self) -> Result<Option<ArtifactRepairAttemptState>> {
        persistence::read_attempt_record(&self.connection.borrow()).map(|record| {
            record.map(|record| ArtifactRepairAttemptState {
                attempt_count: record.attempt_count,
                phase: record.phase,
                started_at: record.started_at,
                next_retry_at: record.next_retry_at,
                last_error_kind: record.last_error_kind,
            })
        })
    }
}

#[cfg(test)]
use persistence::retry_at;

#[cfg(test)]
#[path = "artifact_repair_attempt_tests.rs"]
mod tests;
