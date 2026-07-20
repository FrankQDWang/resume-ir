use rusqlite::{params, Connection};

use crate::{
    ArtifactRepairKey, ContentDigest, ImportProcessingContractId, MetaStoreError, Result,
    UnixTimestamp,
};

pub(super) fn block_current_head(
    connection: &Connection,
    generation: Option<&str>,
    visible_epoch: u64,
    now: UnixTimestamp,
) -> Result<bool> {
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE state_key = 'default' AND service_state = 'ready'
               AND repair_reason IS NULL AND generation IS ?2 AND visible_epoch = ?3",
            params![
                now.as_unix_seconds(),
                generation,
                i64::try_from(visible_epoch).map_err(|_| MetaStoreError::storage_invariant())?,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(changed == 1)
}

pub(super) fn settle_migration_attempt(
    connection: &Connection,
    contract_id: &ImportProcessingContractId,
    barrier_digest: &ContentDigest,
    attempt_id: &ContentDigest,
    attempt_count: u8,
    now: UnixTimestamp,
) -> Result<bool> {
    connection
        .execute(
            "UPDATE migration_rebuild_publication_attempt
             SET phase = 'terminal', next_retry_at_seconds = NULL,
                 last_error_class = 'cleanup', updated_at_seconds = ?1
             WHERE state_key = 'default' AND processing_contract_id = ?2
               AND barrier_digest = ?3 AND attempt_id = ?4
               AND attempt_count = ?5 AND phase = 'running'",
            params![
                now.as_unix_seconds(),
                contract_id.as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
        )
        .map(|changed| changed == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn block_migration_head(
    connection: &Connection,
    contract_id: &ImportProcessingContractId,
    now: UnixTimestamp,
) -> Result<bool> {
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE state_key = 'default' AND service_state = 'repairing'
               AND repair_reason = 'migration_rebuild' AND generation IS NULL
               AND EXISTS (
                   SELECT 1 FROM migration_rebuild_contract_state AS contract
                   WHERE contract.state_key = 'default' AND contract.active_contract_id = ?2
               )",
            params![now.as_unix_seconds(), contract_id.as_str()],
        )
        .map(|changed| changed == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn settle_artifact_attempt(
    connection: &Connection,
    key: &ArtifactRepairKey,
    attempt_id: &ContentDigest,
    attempt_count: u8,
    now: UnixTimestamp,
) -> Result<bool> {
    connection
        .execute(
            "UPDATE artifact_repair_attempt
             SET phase = 'terminal', next_retry_at_seconds = NULL,
                 last_error_kind = 'cleanup', updated_at_seconds = ?1
             WHERE state_key = 'default' AND generation = ?2
               AND publication_fingerprint = ?3 AND visible_epoch = ?4
               AND attempt_id = ?5 AND attempt_count = ?6 AND phase = 'running'",
            params![
                now.as_unix_seconds(),
                key.generation(),
                key.publication_fingerprint().as_str(),
                i64::try_from(key.visible_epoch())
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
        )
        .map(|changed| changed == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn block_artifact_head(
    connection: &Connection,
    key: &ArtifactRepairKey,
    now: UnixTimestamp,
) -> Result<bool> {
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repair_blocked', repair_reason = 'runtime_invariant',
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE state_key = 'default' AND service_state = 'repairing'
               AND repair_reason = 'artifact_unavailable' AND generation = ?2
               AND visible_epoch = ?3 AND EXISTS (
                   SELECT 1 FROM artifact_repair_context AS context
                   WHERE context.state_key = 'default' AND context.generation = ?2
                     AND context.publication_fingerprint = ?4
                     AND context.visible_epoch = ?3
               )",
            params![
                now.as_unix_seconds(),
                key.generation(),
                i64::try_from(key.visible_epoch())
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                key.publication_fingerprint().as_str(),
            ],
        )
        .map(|changed| changed == 1)
        .map_err(MetaStoreError::storage)
}
