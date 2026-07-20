use rusqlite::{params, TransactionBehavior};

use crate::{ContentDigest, MetaStoreError, OwnedMetaStore, Result, UnixTimestamp};

use super::{
    persistence::{
        block_exact_head, delete_exact_attempt, error_kind_to_storage,
        exact_repair_context_matches, exact_running_attempt_matches,
        exact_terminal_blocked_repair_context_matches, random_attempt_id, read_attempt_record,
        retry_at, AttemptCasKey,
    },
    ArtifactRepairAttempt, ArtifactRepairAttemptAcquire, ArtifactRepairAttemptCancellationOutcome,
    ArtifactRepairAttemptFailure, ArtifactRepairAttemptFailureOutcome, ArtifactRepairAttemptPhase,
    ArtifactRepairKey, ArtifactRepairRetrySnapshot, MAX_ATTEMPTS,
};

pub(super) fn acquire_attempt(
    store: &OwnedMetaStore,
    active_attempt_id: Option<&ContentDigest>,
    key: &ArtifactRepairKey,
    now: UnixTimestamp,
) -> Result<ArtifactRepairAttemptAcquire> {
    let epoch =
        i64::try_from(key.visible_epoch).map_err(|_| MetaStoreError::storage_invariant())?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    crate::search_publication::ensure_no_pending_retirement(&transaction)?;
    if !exact_repair_context_matches(
        &transaction,
        &key.generation,
        &key.publication_fingerprint,
        epoch,
    )? {
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(ArtifactRepairAttemptAcquire::Superseded);
    }

    let existing = read_attempt_record(&transaction)?;
    if let Some(existing) = existing.as_ref() {
        let same_context = existing.generation == key.generation
            && existing.publication_fingerprint == key.publication_fingerprint
            && existing.visible_epoch == key.visible_epoch;
        if !same_context {
            transaction
                .execute(
                    "DELETE FROM artifact_repair_attempt WHERE state_key = 'default'",
                    [],
                )
                .map_err(MetaStoreError::storage)?;
        } else {
            match existing.phase {
                ArtifactRepairAttemptPhase::Running => {
                    if active_attempt_id == Some(&existing.attempt_id) {
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(ArtifactRepairAttemptAcquire::InProgress);
                    }
                    if active_attempt_id.is_some() {
                        return Err(MetaStoreError::storage_invariant());
                    }
                    let next_retry_at = retry_at(now, existing.attempt_count);
                    let changed = transaction
                        .execute(
                            "UPDATE artifact_repair_attempt
                             SET phase = 'retry_wait', next_retry_at_seconds = ?1,
                                 last_error_kind = 'interrupted', updated_at_seconds = ?2
                             WHERE state_key = 'default' AND generation = ?3
                               AND publication_fingerprint = ?4 AND visible_epoch = ?5
                               AND attempt_id = ?6 AND attempt_count = ?7
                               AND phase = 'running'",
                            params![
                                next_retry_at.as_unix_seconds(),
                                now.as_unix_seconds(),
                                key.generation,
                                key.publication_fingerprint.as_str(),
                                epoch,
                                existing.attempt_id.as_str(),
                                i64::from(existing.attempt_count),
                            ],
                        )
                        .map_err(MetaStoreError::storage)?;
                    if changed != 1 {
                        return Err(MetaStoreError::storage_invariant());
                    }
                    let exhausted = existing.attempt_count >= MAX_ATTEMPTS;
                    let blocked = exhausted
                        .then(|| {
                            block_exact_head(
                                &transaction,
                                &AttemptCasKey::from_record(existing),
                                now,
                            )
                        })
                        .transpose()?
                        .unwrap_or(false);
                    transaction.commit().map_err(MetaStoreError::storage)?;
                    return Ok(if exhausted {
                        if blocked {
                            ArtifactRepairAttemptAcquire::RepairBlocked
                        } else {
                            ArtifactRepairAttemptAcquire::Superseded
                        }
                    } else {
                        ArtifactRepairAttemptAcquire::NotDue
                    });
                }
                ArtifactRepairAttemptPhase::RetryWait => {
                    let next_retry_at = existing
                        .next_retry_at
                        .ok_or_else(MetaStoreError::storage_invariant)?;
                    if next_retry_at != retry_at(existing.updated_at, existing.attempt_count) {
                        return Err(MetaStoreError::storage_invariant());
                    }
                    if now < existing.updated_at {
                        let rebased_retry_at = retry_at(now, existing.attempt_count);
                        let changed = transaction
                            .execute(
                                "UPDATE artifact_repair_attempt
                                 SET next_retry_at_seconds = ?1, updated_at_seconds = ?2
                                 WHERE state_key = 'default' AND generation = ?3
                                   AND publication_fingerprint = ?4 AND visible_epoch = ?5
                                   AND attempt_id = ?6 AND attempt_count = ?7
                                   AND phase = 'retry_wait' AND updated_at_seconds = ?8
                                   AND next_retry_at_seconds = ?9",
                                params![
                                    rebased_retry_at.as_unix_seconds(),
                                    now.as_unix_seconds(),
                                    key.generation,
                                    key.publication_fingerprint.as_str(),
                                    epoch,
                                    existing.attempt_id.as_str(),
                                    i64::from(existing.attempt_count),
                                    existing.updated_at.as_unix_seconds(),
                                    next_retry_at.as_unix_seconds(),
                                ],
                            )
                            .map_err(MetaStoreError::storage)?;
                        if changed != 1 {
                            return Err(MetaStoreError::storage_invariant());
                        }
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(ArtifactRepairAttemptAcquire::NotDue);
                    }
                    if now.as_unix_seconds() < next_retry_at.as_unix_seconds() {
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(ArtifactRepairAttemptAcquire::NotDue);
                    }
                    if existing.attempt_count >= MAX_ATTEMPTS {
                        let blocked = block_exact_head(
                            &transaction,
                            &AttemptCasKey::from_record(existing),
                            now,
                        )?;
                        transaction.commit().map_err(MetaStoreError::storage)?;
                        return Ok(if blocked {
                            ArtifactRepairAttemptAcquire::RepairBlocked
                        } else {
                            ArtifactRepairAttemptAcquire::Superseded
                        });
                    }
                }
                ArtifactRepairAttemptPhase::Terminal => {
                    return Err(MetaStoreError::storage_invariant());
                }
            }
        }
    }

    let prior_retry = existing
        .as_ref()
        .filter(|record| {
            record.generation == key.generation
                && record.publication_fingerprint == key.publication_fingerprint
                && record.visible_epoch == key.visible_epoch
                && record.phase == ArtifactRepairAttemptPhase::RetryWait
        })
        .map(|record| {
            Ok(ArtifactRepairRetrySnapshot {
                attempt_count: record.attempt_count,
                started_at: record.started_at,
                next_retry_at: record
                    .next_retry_at
                    .ok_or_else(MetaStoreError::storage_invariant)?,
                last_error_kind: record
                    .last_error_kind
                    .ok_or_else(MetaStoreError::storage_invariant)?,
                updated_at: record.updated_at,
            })
        })
        .transpose()?;
    let attempt_count = prior_retry
        .as_ref()
        .map_or(1, |prior| prior.attempt_count.saturating_add(1));
    if attempt_count > MAX_ATTEMPTS {
        return Err(MetaStoreError::storage_invariant());
    }
    let attempt_id = random_attempt_id()?;
    transaction
        .execute(
            "INSERT INTO artifact_repair_attempt (
                state_key, generation, publication_fingerprint, visible_epoch,
                attempt_id, attempt_count, phase, started_at_seconds,
                next_retry_at_seconds, last_error_kind, updated_at_seconds
             ) VALUES ('default', ?1, ?2, ?3, ?4, ?5, 'running', ?6,
                       NULL, NULL, ?6)
             ON CONFLICT(state_key) DO UPDATE SET
                generation = excluded.generation,
                publication_fingerprint = excluded.publication_fingerprint,
                visible_epoch = excluded.visible_epoch,
                attempt_id = excluded.attempt_id,
                attempt_count = excluded.attempt_count,
                phase = excluded.phase,
                started_at_seconds = excluded.started_at_seconds,
                next_retry_at_seconds = NULL,
                last_error_kind = NULL,
                updated_at_seconds = excluded.updated_at_seconds",
            params![
                key.generation,
                key.publication_fingerprint.as_str(),
                epoch,
                attempt_id.as_str(),
                i64::from(attempt_count),
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(ArtifactRepairAttemptAcquire::Started(
        ArtifactRepairAttempt {
            key: key.clone(),
            attempt_id,
            attempt_count,
            prior_retry,
        },
    ))
}

pub(super) fn finish_attempt_failure(
    store: &OwnedMetaStore,
    attempt: &ArtifactRepairAttempt,
    failure: ArtifactRepairAttemptFailure,
    now: UnixTimestamp,
) -> Result<ArtifactRepairAttemptFailureOutcome> {
    let (error_kind, terminal) = match failure {
        ArtifactRepairAttemptFailure::Retryable(error_kind) => (error_kind, false),
        ArtifactRepairAttemptFailure::Terminal(error_kind) => (error_kind, true),
    };
    let epoch = i64::try_from(attempt.key.visible_epoch)
        .map_err(|_| MetaStoreError::storage_invariant())?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    let persisted_attempt = read_attempt_record(&transaction)?;
    let exact_attempt = persisted_attempt.as_ref().is_some_and(|persisted| {
        persisted.generation == attempt.key.generation
            && persisted.publication_fingerprint == attempt.key.publication_fingerprint
            && persisted.visible_epoch == attempt.key.visible_epoch
            && persisted.attempt_id == attempt.attempt_id
            && persisted.attempt_count == attempt.attempt_count
    });
    let exact_running_attempt = exact_attempt
        && persisted_attempt
            .as_ref()
            .is_some_and(|persisted| persisted.phase == ArtifactRepairAttemptPhase::Running);
    let exact_terminal_attempt = exact_attempt
        && persisted_attempt
            .as_ref()
            .is_some_and(|persisted| persisted.phase == ArtifactRepairAttemptPhase::Terminal);
    let exact_repairing_head = exact_running_attempt
        && exact_repair_context_matches(
            &transaction,
            &attempt.key.generation,
            &attempt.key.publication_fingerprint,
            epoch,
        )?;
    let exact_terminal_block = terminal
        && exact_terminal_attempt
        && exact_terminal_blocked_repair_context_matches(
            &transaction,
            &attempt.key.generation,
            &attempt.key.publication_fingerprint,
            epoch,
        )?;
    if exact_terminal_block {
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(ArtifactRepairAttemptFailureOutcome::RepairBlocked);
    }
    if !exact_repairing_head {
        if exact_running_attempt {
            delete_exact_attempt(&transaction, attempt, epoch)?;
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(ArtifactRepairAttemptFailureOutcome::Superseded);
    }
    let changed = transaction
        .execute(
            "UPDATE artifact_repair_attempt
             SET phase = 'retry_wait', next_retry_at_seconds = ?1,
                 last_error_kind = ?2, updated_at_seconds = ?3
             WHERE state_key = 'default' AND generation = ?4
               AND publication_fingerprint = ?5 AND visible_epoch = ?6
               AND attempt_id = ?7 AND attempt_count = ?8 AND phase = 'running'",
            params![
                retry_at(now, attempt.attempt_count).as_unix_seconds(),
                error_kind_to_storage(error_kind),
                now.as_unix_seconds(),
                attempt.key.generation,
                attempt.key.publication_fingerprint.as_str(),
                epoch,
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    if terminal || attempt.attempt_count >= MAX_ATTEMPTS {
        let blocked = block_exact_head(&transaction, &AttemptCasKey::from_attempt(attempt), now)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(if blocked {
            ArtifactRepairAttemptFailureOutcome::RepairBlocked
        } else {
            ArtifactRepairAttemptFailureOutcome::Superseded
        });
    }
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(ArtifactRepairAttemptFailureOutcome::RetryScheduled)
}

pub(super) fn cancel_attempt(
    store: &OwnedMetaStore,
    attempt: &ArtifactRepairAttempt,
) -> Result<ArtifactRepairAttemptCancellationOutcome> {
    let epoch = i64::try_from(attempt.key.visible_epoch)
        .map_err(|_| MetaStoreError::storage_invariant())?;
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    if !exact_running_attempt_matches(&transaction, attempt, epoch)? {
        transaction.commit().map_err(MetaStoreError::storage)?;
        return Ok(ArtifactRepairAttemptCancellationOutcome::Superseded);
    }
    let restored_attempt_id = attempt
        .prior_retry
        .as_ref()
        .map(|_| random_attempt_id())
        .transpose()?;
    let changed = if let Some(prior) = &attempt.prior_retry {
        transaction
            .execute(
                "UPDATE artifact_repair_attempt
                 SET attempt_id = ?1, attempt_count = ?2, phase = 'retry_wait',
                     started_at_seconds = ?3, next_retry_at_seconds = ?4,
                     last_error_kind = ?5, updated_at_seconds = ?6
                 WHERE state_key = 'default' AND generation = ?7
                   AND publication_fingerprint = ?8 AND visible_epoch = ?9
                   AND attempt_id = ?10 AND attempt_count = ?11 AND phase = 'running'",
                params![
                    restored_attempt_id
                        .as_ref()
                        .ok_or_else(MetaStoreError::storage_invariant)?
                        .as_str(),
                    i64::from(prior.attempt_count),
                    prior.started_at.as_unix_seconds(),
                    prior.next_retry_at.as_unix_seconds(),
                    error_kind_to_storage(prior.last_error_kind),
                    prior.updated_at.as_unix_seconds(),
                    attempt.key.generation,
                    attempt.key.publication_fingerprint.as_str(),
                    epoch,
                    attempt.attempt_id.as_str(),
                    i64::from(attempt.attempt_count),
                ],
            )
            .map_err(MetaStoreError::storage)?
    } else {
        delete_exact_attempt(&transaction, attempt, epoch)?
    };
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(if changed == 1 {
        ArtifactRepairAttemptCancellationOutcome::Restored
    } else {
        ArtifactRepairAttemptCancellationOutcome::Superseded
    })
}
