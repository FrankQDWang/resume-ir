use std::{fmt, str::FromStr};

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::migration_rebuild_barrier::{
    migration_rebuild_barrier_token_matches, migration_rebuild_terminal_block_token_matches,
    MigrationRebuildBarrierToken,
};
use crate::{
    ContentDigest, ImportProcessingContractId, MetaStoreError, MetadataStore, MetadataStoreAccess,
    OwnedMetaStore, Result, SearchPublicationSession, UnixTimestamp,
};

const MAX_ATTEMPTS: u8 = 5;
const RETRY_DELAYS_SECONDS: [i64; MAX_ATTEMPTS as usize] = [1, 4, 15, 30, 60];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationRebuildPublicationErrorClass {
    FullText,
    Vector,
    Metadata,
    Cleanup,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationRebuildPublicationFailure {
    Retryable(MigrationRebuildPublicationErrorClass),
    Terminal(MigrationRebuildPublicationErrorClass),
}

#[derive(Clone, PartialEq, Eq)]
pub struct MigrationRebuildPublicationAttempt {
    processing_contract_id: ImportProcessingContractId,
    barrier: MigrationRebuildBarrierToken,
    barrier_digest: ContentDigest,
    attempt_id: ContentDigest,
    attempt_count: u8,
}

impl fmt::Debug for MigrationRebuildPublicationAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationRebuildPublicationAttempt")
            .field("processing_contract_id", &self.processing_contract_id)
            .field("barrier", &self.barrier)
            .field("barrier_digest", &"<redacted>")
            .field("attempt_id", &"<redacted>")
            .field("attempt_count", &self.attempt_count)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationRebuildPublicationAttemptAcquire {
    Started(MigrationRebuildPublicationAttempt),
    InProgress,
    NotDue,
    RepairBlocked,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationRebuildPublicationAttemptFailureOutcome {
    RetryScheduled,
    RepairBlocked,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationRebuildPublicationAttemptPhase {
    Running,
    RetryWait,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationRebuildPublicationAttemptState {
    pub attempt_count: u8,
    pub phase: MigrationRebuildPublicationAttemptPhase,
    pub started_at: UnixTimestamp,
    pub next_retry_at: Option<UnixTimestamp>,
    pub last_error_class: Option<MigrationRebuildPublicationErrorClass>,
}

struct MigrationRebuildPublicationAttemptRecord {
    processing_contract_id: ImportProcessingContractId,
    barrier_digest: ContentDigest,
    attempt_id: ContentDigest,
    attempt_count: u8,
    phase: MigrationRebuildPublicationAttemptPhase,
    started_at: UnixTimestamp,
    next_retry_at: Option<UnixTimestamp>,
    last_error_class: Option<MigrationRebuildPublicationErrorClass>,
}

impl SearchPublicationSession {
    /// Reserves one durable publication attempt only while the supplied
    /// all-root barrier remains closed. The persisted retry deadline prevents
    /// process restarts or fast worker ticks from bypassing the attempt budget.
    pub fn acquire_migration_rebuild_publication_attempt(
        &mut self,
        barrier: &MigrationRebuildBarrierToken,
        now: UnixTimestamp,
    ) -> Result<MigrationRebuildPublicationAttemptAcquire> {
        let outcome = acquire_publication_attempt(
            self.owned_store(),
            self.active_attempt_id(),
            barrier,
            now,
        )?;
        if let MigrationRebuildPublicationAttemptAcquire::Started(attempt) = &outcome {
            self.set_active_attempt_id(attempt.attempt_id.clone());
        }
        Ok(outcome)
    }

    pub fn finish_migration_rebuild_publication_attempt_failure(
        &mut self,
        attempt: &MigrationRebuildPublicationAttempt,
        failure: MigrationRebuildPublicationFailure,
        now: UnixTimestamp,
    ) -> Result<MigrationRebuildPublicationAttemptFailureOutcome> {
        let outcome =
            finish_publication_attempt_failure(self.owned_store(), attempt, failure, now)?;
        self.clear_active_attempt_if(&attempt.attempt_id);
        Ok(outcome)
    }

    /// Abandons an exact attempt whose publication CAS was superseded by a
    /// legitimate barrier/head change. Supersession is not a storage failure
    /// and therefore cannot consume the bounded retry budget.
    pub fn abandon_migration_rebuild_publication_attempt(
        &mut self,
        attempt: &MigrationRebuildPublicationAttempt,
    ) -> Result<MigrationRebuildPublicationAttemptFailureOutcome> {
        let outcome = abandon_publication_attempt(self.owned_store(), attempt)?;
        self.clear_active_attempt_if(&attempt.attempt_id);
        Ok(outcome)
    }
}

fn acquire_publication_attempt(
    store: &OwnedMetaStore,
    active_attempt_id: Option<&ContentDigest>,
    barrier: &MigrationRebuildBarrierToken,
    now: UnixTimestamp,
) -> Result<MigrationRebuildPublicationAttemptAcquire> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    crate::search_publication::ensure_no_pending_retirement(&transaction)?;
    if !migration_rebuild_barrier_token_matches(&transaction, barrier)? {
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(MigrationRebuildPublicationAttemptAcquire::Superseded);
    }

    let barrier_digest = barrier.identity_digest();
    let existing = read_attempt_state(&transaction)?;
    if let Some(existing) = existing.as_ref() {
        if &existing.processing_contract_id != barrier.processing_contract_id() {
            return Err(MetaStoreError::storage_invariant());
        }
        if existing.barrier_digest == barrier_digest {
            match existing.phase {
                MigrationRebuildPublicationAttemptPhase::Running => {
                    if active_attempt_id == Some(&existing.attempt_id) {
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(MigrationRebuildPublicationAttemptAcquire::InProgress);
                    }
                    if active_attempt_id.is_some() {
                        return Err(MetaStoreError::storage_invariant());
                    }
                    let next_retry_at = retry_at(now, existing.attempt_count);
                    transaction
                        .execute(
                            "UPDATE migration_rebuild_publication_attempt
                                 SET phase = 'retry_wait', next_retry_at_seconds = ?1,
                                     last_error_class = 'interrupted',
                                     updated_at_seconds = ?2
                                 WHERE state_key = 'default' AND attempt_id = ?3",
                            params![
                                next_retry_at.as_unix_seconds(),
                                now.as_unix_seconds(),
                                existing.attempt_id.as_str(),
                            ],
                        )
                        .map_err(MetaStoreError::storage)?;
                    let exhausted = existing.attempt_count >= MAX_ATTEMPTS;
                    let blocked = exhausted
                        .then(|| block_unpublished_migration_head(&transaction, now))
                        .transpose()?
                        .unwrap_or(false);
                    transaction.commit().map_err(MetaStoreError::storage)?;
                    return Ok(if exhausted {
                        if blocked {
                            MigrationRebuildPublicationAttemptAcquire::RepairBlocked
                        } else {
                            MigrationRebuildPublicationAttemptAcquire::Superseded
                        }
                    } else {
                        MigrationRebuildPublicationAttemptAcquire::NotDue
                    });
                }
                MigrationRebuildPublicationAttemptPhase::RetryWait => {
                    let next_retry_at = existing
                        .next_retry_at
                        .ok_or_else(MetaStoreError::storage_invariant)?;
                    if now.as_unix_seconds() < next_retry_at.as_unix_seconds() {
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(MigrationRebuildPublicationAttemptAcquire::NotDue);
                    }
                    if existing.attempt_count >= MAX_ATTEMPTS {
                        let blocked = block_unpublished_migration_head(&transaction, now)?;
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(if blocked {
                            MigrationRebuildPublicationAttemptAcquire::RepairBlocked
                        } else {
                            MigrationRebuildPublicationAttemptAcquire::Superseded
                        });
                    }
                }
                MigrationRebuildPublicationAttemptPhase::Terminal => {
                    return Err(MetaStoreError::storage_invariant());
                }
            }
        }
    }

    let attempt_count = existing
        .as_ref()
        .filter(|record| record.barrier_digest == barrier_digest)
        .map_or(1, |record| record.attempt_count.saturating_add(1));
    let attempt_id = random_attempt_id()?;
    transaction
        .execute(
            "INSERT INTO migration_rebuild_publication_attempt (
                    state_key, processing_contract_id, barrier_digest, attempt_id,
                    attempt_count, phase, started_at_seconds,
                    next_retry_at_seconds, last_error_class, updated_at_seconds
                 ) VALUES ('default', ?1, ?2, ?3, ?4, 'running', ?5,
                           NULL, NULL, ?5)
                 ON CONFLICT(state_key) DO UPDATE SET
                    processing_contract_id = excluded.processing_contract_id,
                    barrier_digest = excluded.barrier_digest,
                    attempt_id = excluded.attempt_id,
                    attempt_count = excluded.attempt_count,
                    phase = excluded.phase,
                    started_at_seconds = excluded.started_at_seconds,
                    next_retry_at_seconds = excluded.next_retry_at_seconds,
                    last_error_class = NULL,
                    updated_at_seconds = excluded.updated_at_seconds",
            params![
                barrier.processing_contract_id().as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                i64::from(attempt_count),
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(MigrationRebuildPublicationAttemptAcquire::Started(
        MigrationRebuildPublicationAttempt {
            processing_contract_id: barrier.processing_contract_id().clone(),
            barrier: barrier.clone(),
            barrier_digest,
            attempt_id,
            attempt_count,
        },
    ))
}

fn finish_publication_attempt_failure(
    store: &OwnedMetaStore,
    attempt: &MigrationRebuildPublicationAttempt,
    failure: MigrationRebuildPublicationFailure,
    now: UnixTimestamp,
) -> Result<MigrationRebuildPublicationAttemptFailureOutcome> {
    let (error_class, terminal) = match failure {
        MigrationRebuildPublicationFailure::Retryable(error_class) => (error_class, false),
        MigrationRebuildPublicationFailure::Terminal(error_class) => (error_class, true),
    };
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    let exact_attempt = transaction
        .query_row(
            "SELECT EXISTS (
                     SELECT 1
                     FROM migration_rebuild_publication_attempt AS attempt
                     JOIN migration_rebuild_contract_state AS rebuild
                       ON rebuild.state_key = attempt.state_key
                      AND rebuild.active_contract_id = attempt.processing_contract_id
                     WHERE attempt.state_key = 'default'
                       AND attempt.processing_contract_id = ?1
                       AND attempt.barrier_digest = ?2
                       AND attempt.attempt_id = ?3
                       AND attempt.attempt_count = ?4
                       AND attempt.phase = 'running'
                 )",
            params![
                attempt.processing_contract_id.as_str(),
                attempt.barrier_digest.as_str(),
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        == 1;
    let exact_repairing_head =
        exact_attempt && migration_rebuild_barrier_token_matches(&transaction, &attempt.barrier)?;
    let exact_terminal_block = terminal
        && exact_attempt
        && migration_rebuild_terminal_block_token_matches(&transaction, &attempt.barrier)?;
    if !exact_repairing_head && !exact_terminal_block {
        if exact_attempt {
            delete_exact_attempt(&transaction, attempt)?;
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(MigrationRebuildPublicationAttemptFailureOutcome::Superseded);
    }

    let changed = transaction
        .execute(
            "UPDATE migration_rebuild_publication_attempt
                 SET phase = 'retry_wait', last_error_class = ?1,
                     next_retry_at_seconds = ?2, updated_at_seconds = ?3
                 WHERE state_key = 'default' AND processing_contract_id = ?4
                   AND barrier_digest = ?5 AND attempt_id = ?6
                   AND attempt_count = ?7",
            params![
                error_class_to_storage(error_class),
                retry_at(now, attempt.attempt_count).as_unix_seconds(),
                now.as_unix_seconds(),
                attempt.processing_contract_id.as_str(),
                attempt.barrier_digest.as_str(),
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }

    if terminal || attempt.attempt_count >= MAX_ATTEMPTS {
        let blocked = exact_terminal_block || block_unpublished_migration_head(&transaction, now)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(if blocked {
            MigrationRebuildPublicationAttemptFailureOutcome::RepairBlocked
        } else {
            MigrationRebuildPublicationAttemptFailureOutcome::Superseded
        });
    }
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(MigrationRebuildPublicationAttemptFailureOutcome::RetryScheduled)
}

fn abandon_publication_attempt(
    store: &OwnedMetaStore,
    attempt: &MigrationRebuildPublicationAttempt,
) -> Result<MigrationRebuildPublicationAttemptFailureOutcome> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    let _ = delete_exact_attempt(&transaction, attempt)?;
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(MigrationRebuildPublicationAttemptFailureOutcome::Superseded)
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn migration_rebuild_publication_attempt_state(
        &self,
    ) -> Result<Option<MigrationRebuildPublicationAttemptState>> {
        read_attempt_state(&self.connection.borrow()).map(|state| {
            state.map(|record| MigrationRebuildPublicationAttemptState {
                attempt_count: record.attempt_count,
                phase: record.phase,
                started_at: record.started_at,
                next_retry_at: record.next_retry_at,
                last_error_class: record.last_error_class,
            })
        })
    }
}

fn read_attempt_state(
    connection: &rusqlite::Connection,
) -> Result<Option<MigrationRebuildPublicationAttemptRecord>> {
    connection
        .query_row(
            "SELECT processing_contract_id, barrier_digest, attempt_id, attempt_count,
                    phase, started_at_seconds, next_retry_at_seconds, last_error_class
             FROM migration_rebuild_publication_attempt WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(
            |(
                contract_id,
                barrier_digest,
                attempt_id,
                attempt_count,
                phase,
                started_at,
                next_retry_at,
                error_class,
            )| {
                let attempt_id = ContentDigest::from_str(&attempt_id)
                    .map_err(|_| MetaStoreError::storage_invariant())?;
                Ok(MigrationRebuildPublicationAttemptRecord {
                    processing_contract_id: contract_id.parse()?,
                    barrier_digest: ContentDigest::from_str(&barrier_digest)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    attempt_id,
                    attempt_count: u8::try_from(attempt_count)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    phase: phase_from_storage(&phase)?,
                    started_at: UnixTimestamp::from_unix_seconds(started_at),
                    next_retry_at: next_retry_at.map(UnixTimestamp::from_unix_seconds),
                    last_error_class: error_class
                        .as_deref()
                        .map(error_class_from_storage)
                        .transpose()?,
                })
            },
        )
        .transpose()
}

fn random_attempt_id() -> Result<ContentDigest> {
    let mut entropy = [0_u8; 32];
    getrandom::getrandom(&mut entropy).map_err(|_| MetaStoreError::random())?;
    Ok(ContentDigest::from_bytes(&entropy))
}

fn delete_exact_attempt(
    connection: &rusqlite::Connection,
    attempt: &MigrationRebuildPublicationAttempt,
) -> Result<usize> {
    connection
        .execute(
            "DELETE FROM migration_rebuild_publication_attempt
             WHERE state_key = 'default' AND processing_contract_id = ?1
               AND barrier_digest = ?2 AND attempt_id = ?3
               AND attempt_count = ?4",
            params![
                attempt.processing_contract_id.as_str(),
                attempt.barrier_digest.as_str(),
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
        )
        .map_err(MetaStoreError::storage)
}

fn block_unpublished_migration_head(
    connection: &rusqlite::Connection,
    now: UnixTimestamp,
) -> Result<bool> {
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE state_key = 'default' AND service_state = 'repairing'
               AND repair_reason = 'migration_rebuild' AND generation IS NULL",
            params![now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(changed == 1)
}

fn retry_at(now: UnixTimestamp, attempt_count: u8) -> UnixTimestamp {
    let index = usize::from(attempt_count.saturating_sub(1))
        .min(RETRY_DELAYS_SECONDS.len().saturating_sub(1));
    UnixTimestamp::from_unix_seconds(
        now.as_unix_seconds()
            .saturating_add(RETRY_DELAYS_SECONDS[index]),
    )
}

fn error_class_to_storage(error_class: MigrationRebuildPublicationErrorClass) -> &'static str {
    match error_class {
        MigrationRebuildPublicationErrorClass::FullText => "fulltext",
        MigrationRebuildPublicationErrorClass::Vector => "vector",
        MigrationRebuildPublicationErrorClass::Metadata => "metadata",
        MigrationRebuildPublicationErrorClass::Cleanup => "cleanup",
        MigrationRebuildPublicationErrorClass::Interrupted => "interrupted",
    }
}

fn error_class_from_storage(value: &str) -> Result<MigrationRebuildPublicationErrorClass> {
    match value {
        "fulltext" => Ok(MigrationRebuildPublicationErrorClass::FullText),
        "vector" => Ok(MigrationRebuildPublicationErrorClass::Vector),
        "metadata" => Ok(MigrationRebuildPublicationErrorClass::Metadata),
        "cleanup" => Ok(MigrationRebuildPublicationErrorClass::Cleanup),
        "interrupted" => Ok(MigrationRebuildPublicationErrorClass::Interrupted),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}

fn phase_from_storage(value: &str) -> Result<MigrationRebuildPublicationAttemptPhase> {
    match value {
        "running" => Ok(MigrationRebuildPublicationAttemptPhase::Running),
        "retry_wait" => Ok(MigrationRebuildPublicationAttemptPhase::RetryWait),
        "terminal" => Ok(MigrationRebuildPublicationAttemptPhase::Terminal),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use rusqlite::params;
    use tempfile::{tempdir, TempDir};

    use crate::{
        ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
        ImportProcessingContract, OwnedMetaStore, SearchProjectionDigest,
        SearchProjectionServiceState, SearchPublicationDraft, SearchPublicationOutcome,
        SearchPublicationRetirementFailureOutcome, SearchPublicationSession, SearchRepairReason,
        UnixTimestamp, CLASSIFIER_EPOCH,
    };

    use super::{
        random_attempt_id, MigrationRebuildPublicationAttemptAcquire,
        MigrationRebuildPublicationAttemptFailureOutcome, MigrationRebuildPublicationAttemptPhase,
        MigrationRebuildPublicationErrorClass, MigrationRebuildPublicationFailure,
    };

    #[test]
    fn publication_attempt_backoff_and_budget_survive_store_reopen() {
        let directory = tempdir().unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
        };
        let store = owner.open_store().unwrap();
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let first = started_attempt(&mut session, &barrier, 100);
        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &first,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::FullText,
                    ),
                    UnixTimestamp::from_unix_seconds(100),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::RetryScheduled
        );
        assert_eq!(
            session
                .acquire_migration_rebuild_publication_attempt(
                    &barrier,
                    UnixTimestamp::from_unix_seconds(100),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::NotDue
        );
        drop(session);
        drop(store);

        let store = owner.open_store().unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let second = started_attempt(&mut session, &barrier, 101);
        assert_eq!(
            store
                .migration_rebuild_publication_attempt_state()
                .unwrap()
                .unwrap()
                .attempt_count,
            2
        );
        retry(&mut session, &second, 101);
        let third = started_attempt(&mut session, &barrier, 105);
        retry(&mut session, &third, 105);
        let fourth = started_attempt(&mut session, &barrier, 120);
        retry(&mut session, &fourth, 120);
        let fifth = started_attempt(&mut session, &barrier, 150);
        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &fifth,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::Vector,
                    ),
                    UnixTimestamp::from_unix_seconds(150),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::RepairBlocked
        );
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
    }

    #[test]
    fn cleanup_failure_blocks_on_the_first_closed_barrier_attempt() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(200))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let attempt = started_attempt(&mut session, &barrier, 200);
        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &attempt,
                    MigrationRebuildPublicationFailure::Terminal(
                        MigrationRebuildPublicationErrorClass::Cleanup,
                    ),
                    UnixTimestamp::from_unix_seconds(200),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::RepairBlocked
        );
    }

    #[test]
    fn terminal_retirement_failure_settles_the_exact_already_blocked_attempt_after_reopen() {
        let directory = tempdir().unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
        };
        let store = owner.open_store().unwrap();
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(200))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let _attempt = started_attempt(&mut session, &barrier, 200);
        let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
        assert_eq!(
            session
                .begin_search_publication(&SearchPublicationDraft {
                    generation: "migration-retirement-terminal".to_string(),
                    base_generation: None,
                    expected_visible_epoch: 0,
                    classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                    projection_digest,
                    now: UnixTimestamp::from_unix_seconds(201),
                })
                .unwrap(),
            SearchPublicationOutcome::Applied
        );
        session
            .begin_search_publication_retirement(
                "migration-retirement-terminal",
                UnixTimestamp::from_unix_seconds(202),
                crate::SearchPublicationRetirementPlan {
                    fulltext: crate::SearchArtifactExpectation::MayExist,
                    vector: crate::SearchArtifactExpectation::MayExist,
                },
            )
            .unwrap();
        assert!(session
            .fail_search_publication_retirement_settlement_before_commit_for_test(
                "migration-retirement-terminal",
                UnixTimestamp::from_unix_seconds(203),
            )
            .is_err());
        let rolled_back_head = store.search_projection_state().unwrap();
        assert_eq!(
            rolled_back_head.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            rolled_back_head.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        let rolled_back_attempt = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(
            rolled_back_attempt.phase,
            MigrationRebuildPublicationAttemptPhase::Running
        );
        assert_eq!(rolled_back_attempt.last_error_class, None);
        assert_eq!(
            session
                .block_search_head_after_publication_retirement_failure(
                    "migration-retirement-terminal",
                    UnixTimestamp::from_unix_seconds(203),
                )
                .unwrap(),
            SearchPublicationRetirementFailureOutcome::HeadBlocked
        );

        drop(session);
        drop(store);
        drop(owner);

        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
        };
        let store = owner.open_store().unwrap();
        let blocked = store.search_projection_state().unwrap();
        assert_eq!(
            blocked.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            blocked.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
        assert_eq!(blocked.generation, None);
        assert_eq!(blocked.visible_epoch, 0);
        let settled = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(settled.attempt_count, 1);
        assert_eq!(
            settled.phase,
            MigrationRebuildPublicationAttemptPhase::Terminal
        );
        assert_eq!(
            settled.last_error_class,
            Some(MigrationRebuildPublicationErrorClass::Cleanup)
        );
        let mut session = store.wait_for_search_publication_session().unwrap();
        assert!(session
            .acquire_migration_rebuild_publication_attempt(
                &barrier,
                UnixTimestamp::from_unix_seconds(1_000),
            )
            .is_err());
        assert_eq!(
            store.migration_rebuild_publication_attempt_state().unwrap(),
            Some(settled.clone())
        );
        assert_eq!(
            session
                .block_search_head_after_publication_retirement_failure(
                    "migration-retirement-terminal",
                    UnixTimestamp::from_unix_seconds(204),
                )
                .unwrap(),
            SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
        );
        assert_eq!(store.search_projection_state().unwrap(), blocked);
        assert_eq!(
            store.migration_rebuild_publication_attempt_state().unwrap(),
            Some(settled.clone())
        );

        let mismatched_attempt_id =
            ContentDigest::from_bytes(b"mismatched migration cleanup attempt");
        store
            .connection
            .borrow()
            .execute(
                "UPDATE migration_rebuild_publication_attempt
                 SET attempt_id = ?1 WHERE state_key = 'default'",
                [mismatched_attempt_id.as_str()],
            )
            .unwrap();
        assert_eq!(
            session
                .block_search_head_after_publication_retirement_failure(
                    "migration-retirement-terminal",
                    UnixTimestamp::from_unix_seconds(205),
                )
                .unwrap(),
            SearchPublicationRetirementFailureOutcome::HeadSuperseded
        );
        assert_eq!(store.search_projection_state().unwrap(), blocked);
        assert_eq!(
            store.migration_rebuild_publication_attempt_state().unwrap(),
            Some(settled)
        );
    }

    #[test]
    fn retry_backoff_starts_when_the_attempt_failure_finishes() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let attempt = started_attempt(&mut session, &barrier, 100);

        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &attempt,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::FullText,
                    ),
                    UnixTimestamp::from_unix_seconds(130),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::RetryScheduled
        );
        assert_eq!(
            store
                .migration_rebuild_publication_attempt_state()
                .unwrap()
                .unwrap()
                .next_retry_at,
            Some(UnixTimestamp::from_unix_seconds(131))
        );
        assert_eq!(
            session
                .acquire_migration_rebuild_publication_attempt(
                    &barrier,
                    UnixTimestamp::from_unix_seconds(130),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::NotDue
        );
        let second = started_attempt(&mut session, &barrier, 131);
        assert_eq!(second.attempt_count, 2);
    }

    #[test]
    fn superseded_barrier_abandons_instead_of_consuming_failure_budget() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(200))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let attempt = started_attempt(&mut session, &barrier, 200);
        store
            .connection
            .borrow()
            .execute(
                "INSERT INTO authorized_import_root (
                    canonical_root_path, requested_root_path, root_kind,
                    root_preset, scan_profile, scan_budget_kind,
                    scan_budget_limit, paused, updated_at_seconds
                 ) VALUES ('/synthetic/new-root', '/synthetic/new-root',
                           'explicit', NULL, 'explicit', NULL, NULL, 0, 201)",
                [],
            )
            .unwrap();

        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &attempt,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::FullText,
                    ),
                    UnixTimestamp::from_unix_seconds(202),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::Superseded
        );
        assert!(store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .is_none());

        store
            .connection
            .borrow()
            .execute(
                "UPDATE authorized_import_root SET paused = 1
                 WHERE canonical_root_path = '/synthetic/new-root'",
                [],
            )
            .unwrap();
        let replacement = started_attempt(&mut session, &barrier, 203);
        assert_eq!(replacement.attempt_count, 1);
    }

    #[test]
    fn publication_cas_supersession_clears_the_exact_attempt() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(300))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let attempt = started_attempt(&mut session, &barrier, 300);

        assert_eq!(
            session
                .abandon_migration_rebuild_publication_attempt(&attempt)
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::Superseded
        );
        assert!(store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .is_none());
        assert_eq!(
            started_attempt(&mut session, &barrier, 301).attempt_count,
            1
        );
    }

    #[test]
    fn new_lock_owning_session_recovers_persisted_running_attempt() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let attempt = started_attempt(&mut session, &barrier, 100);
        let attempt_id = attempt.attempt_id.clone();
        drop(attempt);
        drop(session);

        let mut session = store.wait_for_search_publication_session().unwrap();
        assert_eq!(
            session
                .acquire_migration_rebuild_publication_attempt(
                    &barrier,
                    UnixTimestamp::from_unix_seconds(160),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::NotDue
        );
        let recovered = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(recovered.attempt_count, 1);
        assert_eq!(
            recovered.phase,
            MigrationRebuildPublicationAttemptPhase::RetryWait
        );
        assert_eq!(recovered.started_at, UnixTimestamp::from_unix_seconds(100));
        assert_eq!(
            recovered.next_retry_at,
            Some(UnixTimestamp::from_unix_seconds(161))
        );
        assert_eq!(
            recovered.last_error_class,
            Some(MigrationRebuildPublicationErrorClass::Interrupted)
        );

        let replacement = started_attempt(&mut session, &barrier, 161);
        assert_eq!(replacement.attempt_count, 2);
        assert_ne!(replacement.attempt_id, attempt_id);
    }

    #[test]
    fn same_session_running_attempt_is_not_recovered_as_orphan() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract = contract();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
            .unwrap();
        let barrier = store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let _attempt = started_attempt(&mut session, &barrier, 100);

        assert_eq!(
            session
                .acquire_migration_rebuild_publication_attempt(
                    &barrier,
                    UnixTimestamp::from_unix_seconds(160),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::InProgress
        );
        let state = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(
            state.phase,
            MigrationRebuildPublicationAttemptPhase::Running
        );
        assert_eq!(state.next_retry_at, None);
        assert_eq!(state.last_error_class, None);
    }

    #[test]
    fn second_search_publication_session_waits_for_the_live_generation_owner() {
        let fixture = owned_fixture();
        let first = fixture.store.wait_for_search_publication_session().unwrap();
        let waiting_store = fixture.store.open_sibling().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let second = waiting_store.wait_for_search_publication_session().unwrap();
            acquired_tx.send(()).unwrap();
            drop(second);
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());

        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn nested_search_publication_session_fails_closed_on_the_holding_thread() {
        let fixture = owned_fixture();
        let _first = fixture.store.wait_for_search_publication_session().unwrap();

        assert_eq!(
            fixture
                .store
                .wait_for_search_publication_session()
                .unwrap_err()
                .class(),
            crate::MetaStoreErrorClass::StorageInvariant
        );
    }

    #[test]
    fn attempt_id_fences_stale_handle_across_barrier_aba() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract_a = contract();
        let contract_b = alternate_contract();
        store
            .activate_migration_rebuild_contract(&contract_a, UnixTimestamp::from_unix_seconds(200))
            .unwrap();
        let barrier_a = store
            .acquire_migration_rebuild_barrier_token(contract_a.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let stale_attempt = started_attempt(&mut session, &barrier_a, 200);

        store
            .activate_migration_rebuild_contract(&contract_b, UnixTimestamp::from_unix_seconds(201))
            .unwrap();
        let barrier_b = store
            .acquire_migration_rebuild_barrier_token(contract_b.id())
            .unwrap()
            .unwrap();
        let _intermediate = started_attempt(&mut session, &barrier_b, 201);
        store
            .activate_migration_rebuild_contract(&contract_a, UnixTimestamp::from_unix_seconds(202))
            .unwrap();
        let barrier_a_again = store
            .acquire_migration_rebuild_barrier_token(contract_a.id())
            .unwrap()
            .unwrap();
        assert_eq!(barrier_a_again, barrier_a);
        let replacement = started_attempt(&mut session, &barrier_a_again, 202);
        assert_eq!(replacement.attempt_count, 1);
        assert_ne!(replacement.attempt_id, stale_attempt.attempt_id);

        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &stale_attempt,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::FullText,
                    ),
                    UnixTimestamp::from_unix_seconds(203),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::Superseded
        );
        let current = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(current.attempt_count, 1);
        assert_eq!(
            current.phase,
            MigrationRebuildPublicationAttemptPhase::Running
        );
        assert_eq!(current.last_error_class, None);
        retry(&mut session, &replacement, 203);
    }

    #[test]
    fn stale_cleanup_failure_cannot_block_the_superseding_unpublished_head() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract_a = contract();
        let contract_b = alternate_contract();
        store
            .activate_migration_rebuild_contract(&contract_a, UnixTimestamp::from_unix_seconds(300))
            .unwrap();
        let barrier_a = store
            .acquire_migration_rebuild_barrier_token(contract_a.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let stale_attempt = started_attempt(&mut session, &barrier_a, 300);
        store
            .activate_migration_rebuild_contract(&contract_b, UnixTimestamp::from_unix_seconds(301))
            .unwrap();
        let barrier_b = store
            .acquire_migration_rebuild_barrier_token(contract_b.id())
            .unwrap()
            .unwrap();
        let _current_attempt = started_attempt(&mut session, &barrier_b, 301);

        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    &stale_attempt,
                    MigrationRebuildPublicationFailure::Terminal(
                        MigrationRebuildPublicationErrorClass::Cleanup,
                    ),
                    UnixTimestamp::from_unix_seconds(302),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::Superseded
        );
        let projection = store.search_projection_state().unwrap();
        assert_eq!(
            projection.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            projection.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        let current = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(current.attempt_count, 1);
        assert_eq!(
            current.phase,
            MigrationRebuildPublicationAttemptPhase::Running
        );
        assert_eq!(current.last_error_class, None);
    }

    #[test]
    fn stale_publication_cleanup_cannot_block_a_new_null_generation_contract() {
        let fixture = owned_fixture();
        let store = &fixture.store;
        let contract_a = contract();
        let contract_b = alternate_contract();
        store
            .activate_migration_rebuild_contract(&contract_a, UnixTimestamp::from_unix_seconds(300))
            .unwrap();
        let barrier_a = store
            .acquire_migration_rebuild_barrier_token(contract_a.id())
            .unwrap()
            .unwrap();
        let mut session = store.wait_for_search_publication_session().unwrap();
        let _stale_attempt = started_attempt(&mut session, &barrier_a, 300);
        let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
        assert_eq!(
            session
                .begin_search_publication(&SearchPublicationDraft {
                    generation: "contract-a-retirement".to_string(),
                    base_generation: None,
                    expected_visible_epoch: 0,
                    classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                    projection_digest,
                    now: UnixTimestamp::from_unix_seconds(301),
                })
                .unwrap(),
            SearchPublicationOutcome::Applied
        );
        session
            .begin_search_publication_retirement(
                "contract-a-retirement",
                UnixTimestamp::from_unix_seconds(302),
                crate::SearchPublicationRetirementPlan {
                    fulltext: crate::SearchArtifactExpectation::MayExist,
                    vector: crate::SearchArtifactExpectation::MayExist,
                },
            )
            .unwrap();

        store
            .activate_migration_rebuild_contract(&contract_b, UnixTimestamp::from_unix_seconds(303))
            .unwrap();
        let barrier_b = store
            .acquire_migration_rebuild_barrier_token(contract_b.id())
            .unwrap()
            .unwrap();
        assert!(session
            .acquire_migration_rebuild_publication_attempt(
                &barrier_b,
                UnixTimestamp::from_unix_seconds(303),
            )
            .is_err());
        let barrier_b_digest = barrier_b.identity_digest();
        let current_attempt_id = random_attempt_id().unwrap();
        store
            .connection
            .borrow()
            .execute(
                "INSERT INTO migration_rebuild_publication_attempt (
                     state_key, processing_contract_id, barrier_digest, attempt_id,
                     attempt_count, phase, started_at_seconds, next_retry_at_seconds,
                     last_error_class, updated_at_seconds
                 ) VALUES ('default', ?1, ?2, ?3, 1, 'running', 303, NULL, NULL, 303)",
                params![
                    contract_b.id().as_str(),
                    barrier_b_digest.as_str(),
                    current_attempt_id.as_str(),
                ],
            )
            .unwrap();

        assert_eq!(
            session
                .block_search_head_after_publication_retirement_failure(
                    "contract-a-retirement",
                    UnixTimestamp::from_unix_seconds(304),
                )
                .unwrap(),
            SearchPublicationRetirementFailureOutcome::HeadSuperseded
        );
        let head = store.search_projection_state().unwrap();
        assert_eq!(head.service_state, SearchProjectionServiceState::Repairing);
        assert_eq!(
            head.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        assert_eq!(
            store
                .migration_rebuild_publication_attempt_state()
                .unwrap()
                .unwrap()
                .phase,
            MigrationRebuildPublicationAttemptPhase::Running
        );
    }

    fn contract() -> ImportProcessingContract {
        ImportProcessingContract::new(
            "attempt-parser-v1",
            "attempt-ocr-v1",
            "attempt-schema-v28",
            CLASSIFIER_EPOCH,
        )
        .unwrap()
    }

    fn alternate_contract() -> ImportProcessingContract {
        ImportProcessingContract::new(
            "attempt-parser-v2",
            "attempt-ocr-v2",
            "attempt-schema-v28",
            CLASSIFIER_EPOCH,
        )
        .unwrap()
    }

    struct OwnedFixture {
        store: OwnedMetaStore,
        _owner: DataDirectoryOwnerLease,
        _directory: TempDir,
    }

    fn owned_fixture() -> OwnedFixture {
        let directory = tempdir().unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
        };
        let store = owner.open_store().unwrap();
        OwnedFixture {
            store,
            _owner: owner,
            _directory: directory,
        }
    }

    fn started_attempt(
        session: &mut SearchPublicationSession,
        barrier: &crate::MigrationRebuildBarrierToken,
        now: i64,
    ) -> crate::MigrationRebuildPublicationAttempt {
        match session
            .acquire_migration_rebuild_publication_attempt(
                barrier,
                UnixTimestamp::from_unix_seconds(now),
            )
            .unwrap()
        {
            MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
            outcome => panic!("expected started attempt, got {outcome:?}"),
        }
    }

    fn retry(
        session: &mut SearchPublicationSession,
        attempt: &crate::MigrationRebuildPublicationAttempt,
        now: i64,
    ) {
        assert_eq!(
            session
                .finish_migration_rebuild_publication_attempt_failure(
                    attempt,
                    MigrationRebuildPublicationFailure::Retryable(
                        MigrationRebuildPublicationErrorClass::Vector,
                    ),
                    UnixTimestamp::from_unix_seconds(now),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptFailureOutcome::RetryScheduled
        );
    }
}
