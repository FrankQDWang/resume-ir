use std::str::FromStr;

use rusqlite::{params, OptionalExtension};

use crate::{ContentDigest, MetaStoreError, Result, UnixTimestamp};

use super::{
    ArtifactRepairAttempt, ArtifactRepairAttemptErrorKind, ArtifactRepairAttemptPhase,
    ArtifactRepairAttemptRecord, RETRY_DELAYS_SECONDS,
};

pub(super) fn read_attempt_record(
    connection: &rusqlite::Connection,
) -> Result<Option<ArtifactRepairAttemptRecord>> {
    connection
        .query_row(
            "SELECT generation, publication_fingerprint, visible_epoch, attempt_id,
                    attempt_count, phase, started_at_seconds, next_retry_at_seconds,
                    last_error_kind, updated_at_seconds
             FROM artifact_repair_attempt WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(
            |(
                generation,
                fingerprint,
                epoch,
                attempt_id,
                attempt_count,
                phase,
                started_at,
                next_retry_at,
                error_kind,
                updated_at,
            )| {
                Ok(ArtifactRepairAttemptRecord {
                    generation,
                    publication_fingerprint: ContentDigest::from_str(&fingerprint)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    visible_epoch: u64::try_from(epoch)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    attempt_id: ContentDigest::from_str(&attempt_id)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    attempt_count: u8::try_from(attempt_count)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    phase: phase_from_storage(&phase)?,
                    started_at: UnixTimestamp::from_unix_seconds(started_at),
                    next_retry_at: next_retry_at.map(UnixTimestamp::from_unix_seconds),
                    last_error_kind: error_kind
                        .as_deref()
                        .map(error_kind_from_storage)
                        .transpose()?,
                    updated_at: UnixTimestamp::from_unix_seconds(updated_at),
                })
            },
        )
        .transpose()
}

pub(super) fn exact_repair_context_matches(
    connection: &rusqlite::Connection,
    generation: &str,
    publication_fingerprint: &ContentDigest,
    visible_epoch: i64,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM artifact_repair_context AS context
                 JOIN search_projection_state AS head
                   ON head.state_key = context.state_key
                  AND head.generation = context.generation
                  AND head.visible_epoch = context.visible_epoch
                 WHERE context.state_key = 'default'
                   AND context.generation = ?1
                   AND context.publication_fingerprint = ?2
                   AND context.visible_epoch = ?3
                   AND head.service_state = 'repairing'
                   AND head.repair_reason = 'artifact_unavailable'
             )",
            params![generation, publication_fingerprint.as_str(), visible_epoch],
            |row| row.get::<_, i64>(0),
        )
        .map(|matches| matches == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_terminal_blocked_repair_context_matches(
    connection: &rusqlite::Connection,
    generation: &str,
    publication_fingerprint: &ContentDigest,
    visible_epoch: i64,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM artifact_repair_context AS context
                 JOIN search_projection_state AS head
                   ON head.state_key = context.state_key
                  AND head.generation = context.generation
                  AND head.visible_epoch = context.visible_epoch
                 WHERE context.state_key = 'default'
                   AND context.generation = ?1
                   AND context.publication_fingerprint = ?2
                   AND context.visible_epoch = ?3
                   AND head.service_state = 'repair_blocked'
                   AND head.repair_reason = 'runtime_invariant'
             )",
            params![generation, publication_fingerprint.as_str(), visible_epoch],
            |row| row.get::<_, i64>(0),
        )
        .map(|matches| matches == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_running_attempt_matches(
    connection: &rusqlite::Connection,
    attempt: &ArtifactRepairAttempt,
    visible_epoch: i64,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM artifact_repair_attempt
                 WHERE state_key = 'default' AND generation = ?1
                   AND publication_fingerprint = ?2 AND visible_epoch = ?3
                   AND attempt_id = ?4 AND attempt_count = ?5 AND phase = 'running'
             )",
            params![
                attempt.key.generation,
                attempt.key.publication_fingerprint.as_str(),
                visible_epoch,
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matches| matches == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn delete_exact_attempt(
    connection: &rusqlite::Connection,
    attempt: &ArtifactRepairAttempt,
    visible_epoch: i64,
) -> Result<usize> {
    connection
        .execute(
            "DELETE FROM artifact_repair_attempt
             WHERE state_key = 'default' AND generation = ?1
               AND publication_fingerprint = ?2 AND visible_epoch = ?3
               AND attempt_id = ?4 AND attempt_count = ?5 AND phase = 'running'",
            params![
                attempt.key.generation,
                attempt.key.publication_fingerprint.as_str(),
                visible_epoch,
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
        )
        .map_err(MetaStoreError::storage)
}

pub(super) struct AttemptCasKey<'a> {
    generation: &'a str,
    publication_fingerprint: &'a ContentDigest,
    visible_epoch: u64,
    attempt_id: &'a ContentDigest,
    attempt_count: u8,
}

impl<'a> AttemptCasKey<'a> {
    pub(super) fn from_record(record: &'a ArtifactRepairAttemptRecord) -> Self {
        Self {
            generation: &record.generation,
            publication_fingerprint: &record.publication_fingerprint,
            visible_epoch: record.visible_epoch,
            attempt_id: &record.attempt_id,
            attempt_count: record.attempt_count,
        }
    }

    pub(super) fn from_attempt(attempt: &'a ArtifactRepairAttempt) -> Self {
        Self {
            generation: &attempt.key.generation,
            publication_fingerprint: &attempt.key.publication_fingerprint,
            visible_epoch: attempt.key.visible_epoch,
            attempt_id: &attempt.attempt_id,
            attempt_count: attempt.attempt_count,
        }
    }
}

pub(super) fn block_exact_head(
    connection: &rusqlite::Connection,
    attempt: &AttemptCasKey<'_>,
    now: UnixTimestamp,
) -> Result<bool> {
    let visible_epoch =
        i64::try_from(attempt.visible_epoch).map_err(|_| MetaStoreError::storage_invariant())?;
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE state_key = 'default' AND service_state = 'repairing'
               AND repair_reason = 'artifact_unavailable' AND generation = ?2
               AND visible_epoch = ?3
               AND EXISTS (
                   SELECT 1 FROM artifact_repair_context AS context
                   WHERE context.state_key = 'default' AND context.generation = ?2
                     AND context.publication_fingerprint = ?4
                     AND context.visible_epoch = ?3
               )
               AND EXISTS (
                   SELECT 1 FROM artifact_repair_attempt AS attempt
                   WHERE attempt.state_key = 'default' AND attempt.generation = ?2
                     AND attempt.publication_fingerprint = ?4
                     AND attempt.visible_epoch = ?3 AND attempt.attempt_id = ?5
                     AND attempt.attempt_count = ?6 AND attempt.phase = 'retry_wait'
               )",
            params![
                now.as_unix_seconds(),
                attempt.generation,
                visible_epoch,
                attempt.publication_fingerprint.as_str(),
                attempt.attempt_id.as_str(),
                i64::from(attempt.attempt_count),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(changed == 1)
}

pub(super) fn random_attempt_id() -> Result<ContentDigest> {
    let mut entropy = [0_u8; 32];
    getrandom::getrandom(&mut entropy).map_err(|_| MetaStoreError::random())?;
    Ok(ContentDigest::from_bytes(&entropy))
}

pub(super) fn retry_at(now: UnixTimestamp, attempt_count: u8) -> UnixTimestamp {
    let index = usize::from(attempt_count.saturating_sub(1))
        .min(RETRY_DELAYS_SECONDS.len().saturating_sub(1));
    UnixTimestamp::from_unix_seconds(
        now.as_unix_seconds()
            .saturating_add(RETRY_DELAYS_SECONDS[index]),
    )
}

pub(super) fn error_kind_to_storage(error_kind: ArtifactRepairAttemptErrorKind) -> &'static str {
    error_kind.label()
}

fn error_kind_from_storage(value: &str) -> Result<ArtifactRepairAttemptErrorKind> {
    match value {
        "fulltext_publication_busy" => Ok(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy),
        "fulltext_failure" => Ok(ArtifactRepairAttemptErrorKind::FullTextFailure),
        "vector_publication_busy" => Ok(ArtifactRepairAttemptErrorKind::VectorPublicationBusy),
        "vector_failure" => Ok(ArtifactRepairAttemptErrorKind::VectorFailure),
        "metadata_failure" => Ok(ArtifactRepairAttemptErrorKind::MetadataFailure),
        "cleanup" => Ok(ArtifactRepairAttemptErrorKind::Cleanup),
        "interrupted" => Ok(ArtifactRepairAttemptErrorKind::Interrupted),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}

fn phase_from_storage(value: &str) -> Result<ArtifactRepairAttemptPhase> {
    match value {
        "running" => Ok(ArtifactRepairAttemptPhase::Running),
        "retry_wait" => Ok(ArtifactRepairAttemptPhase::RetryWait),
        "terminal" => Ok(ArtifactRepairAttemptPhase::Terminal),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}
