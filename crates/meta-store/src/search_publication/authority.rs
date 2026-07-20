use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    migration_rebuild_barrier::{
        migration_rebuild_barrier_digest_matches, migration_rebuild_barrier_token_matches,
        migration_rebuild_terminal_block_digest_matches,
    },
    ArtifactRepairKey, ContentDigest, ImportProcessingContractId, MetaStoreError,
    MigrationRebuildBarrierToken, Result,
};

use super::{
    model::{SearchPublicationDraft, SearchPublicationFailure},
    validation::publication_error,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PublicationAuthority {
    CurrentHead,
    MigrationRebuild {
        contract_id: ImportProcessingContractId,
        barrier_digest: ContentDigest,
        attempt_id: ContentDigest,
        attempt_count: u8,
    },
    ArtifactRepair {
        key: ArtifactRepairKey,
        attempt_id: ContentDigest,
        attempt_count: u8,
    },
}

pub(super) struct AuthorityStorage<'a> {
    pub(super) kind: &'static str,
    pub(super) contract_id: Option<&'a str>,
    pub(super) barrier_digest: Option<&'a str>,
    pub(super) repair_generation: Option<&'a str>,
    pub(super) repair_fingerprint: Option<&'a str>,
    pub(super) repair_visible_epoch: Option<i64>,
    pub(super) attempt_id: Option<&'a str>,
    pub(super) attempt_count: Option<i64>,
}

struct PersistedAuthorityRow {
    kind: String,
    contract_id: Option<String>,
    barrier_digest: Option<String>,
    repair_generation: Option<String>,
    repair_fingerprint: Option<String>,
    repair_visible_epoch: Option<i64>,
    attempt_id: Option<String>,
    attempt_count: Option<i64>,
}

impl PublicationAuthority {
    pub(super) fn storage(&self) -> Result<AuthorityStorage<'_>> {
        Ok(match self {
            Self::CurrentHead => AuthorityStorage {
                kind: "current_head",
                contract_id: None,
                barrier_digest: None,
                repair_generation: None,
                repair_fingerprint: None,
                repair_visible_epoch: None,
                attempt_id: None,
                attempt_count: None,
            },
            Self::MigrationRebuild {
                contract_id,
                barrier_digest,
                attempt_id,
                attempt_count,
            } => AuthorityStorage {
                kind: "migration_rebuild",
                contract_id: Some(contract_id.as_str()),
                barrier_digest: Some(barrier_digest.as_str()),
                repair_generation: None,
                repair_fingerprint: None,
                repair_visible_epoch: None,
                attempt_id: Some(attempt_id.as_str()),
                attempt_count: Some(i64::from(*attempt_count)),
            },
            Self::ArtifactRepair {
                key,
                attempt_id,
                attempt_count,
            } => AuthorityStorage {
                kind: "artifact_repair",
                contract_id: None,
                barrier_digest: None,
                repair_generation: Some(key.generation()),
                repair_fingerprint: Some(key.publication_fingerprint().as_str()),
                repair_visible_epoch: Some(
                    i64::try_from(key.visible_epoch())
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                ),
                attempt_id: Some(attempt_id.as_str()),
                attempt_count: Some(i64::from(*attempt_count)),
            },
        })
    }
}

pub(super) fn authority_for_begin(
    connection: &Connection,
    active_attempt_id: Option<&ContentDigest>,
    draft: &SearchPublicationDraft,
) -> Result<Option<PublicationAuthority>> {
    let expected_epoch = i64::try_from(draft.expected_visible_epoch)
        .map_err(|_| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
    let (state, generation, visible_epoch, repair_reason) = connection
        .query_row(
            "SELECT service_state, generation, visible_epoch, repair_reason
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    if generation != draft.base_generation || visible_epoch != expected_epoch {
        return Ok(None);
    }
    match (state.as_str(), repair_reason.as_deref()) {
        ("ready", None) if generation.is_some() => Ok(Some(PublicationAuthority::CurrentHead)),
        ("repairing", Some("migration_rebuild")) if generation.is_none() => {
            running_migration_authority(connection, active_attempt_id)
        }
        ("repairing", Some("artifact_unavailable")) if generation.is_some() => {
            running_artifact_authority(
                connection,
                active_attempt_id,
                generation.as_deref(),
                visible_epoch,
            )
        }
        _ => Ok(None),
    }
}

fn running_migration_authority(
    connection: &Connection,
    active_attempt_id: Option<&ContentDigest>,
) -> Result<Option<PublicationAuthority>> {
    let Some(active_attempt_id) = active_attempt_id else {
        return Ok(None);
    };
    let record = connection
        .query_row(
            "SELECT processing_contract_id, barrier_digest, attempt_id, attempt_count
             FROM migration_rebuild_publication_attempt
             WHERE state_key = 'default' AND phase = 'running' AND attempt_id = ?1",
            params![active_attempt_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let Some((contract_id, barrier_digest, attempt_id, attempt_count)) = record else {
        return Ok(None);
    };
    let contract_id = contract_id.parse::<ImportProcessingContractId>()?;
    let barrier_digest = parse_digest(&barrier_digest)?;
    if !migration_rebuild_barrier_digest_matches(connection, &contract_id, &barrier_digest)? {
        return Ok(None);
    }
    Ok(Some(PublicationAuthority::MigrationRebuild {
        contract_id,
        barrier_digest,
        attempt_id: parse_digest(&attempt_id)?,
        attempt_count: u8::try_from(attempt_count)
            .map_err(|_| MetaStoreError::storage_invariant())?,
    }))
}

fn running_artifact_authority(
    connection: &Connection,
    active_attempt_id: Option<&ContentDigest>,
    generation: Option<&str>,
    visible_epoch: i64,
) -> Result<Option<PublicationAuthority>> {
    let Some(active_attempt_id) = active_attempt_id else {
        return Ok(None);
    };
    let record = connection
        .query_row(
            "SELECT attempt.generation, attempt.publication_fingerprint,
                    attempt.visible_epoch, attempt.attempt_id, attempt.attempt_count
             FROM artifact_repair_attempt AS attempt
             JOIN artifact_repair_context AS context ON context.state_key = attempt.state_key
               AND context.generation = attempt.generation
               AND context.publication_fingerprint = attempt.publication_fingerprint
               AND context.visible_epoch = attempt.visible_epoch
             WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
               AND attempt.attempt_id = ?1 AND attempt.generation = ?2
               AND attempt.visible_epoch = ?3",
            params![active_attempt_id.as_str(), generation, visible_epoch],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let Some((generation, fingerprint, epoch, attempt_id, attempt_count)) = record else {
        return Ok(None);
    };
    Ok(Some(PublicationAuthority::ArtifactRepair {
        key: ArtifactRepairKey::new(
            generation,
            parse_digest(&fingerprint)?,
            u64::try_from(epoch).map_err(|_| MetaStoreError::storage_invariant())?,
        ),
        attempt_id: parse_digest(&attempt_id)?,
        attempt_count: u8::try_from(attempt_count)
            .map_err(|_| MetaStoreError::storage_invariant())?,
    }))
}

pub(super) fn authority_matches_commit(
    connection: &Connection,
    generation: &str,
    migration_barrier: Option<&MigrationRebuildBarrierToken>,
) -> Result<bool> {
    let Some(authority) = read_authority(connection, generation)? else {
        return Ok(false);
    };
    match (authority, migration_barrier) {
        (PublicationAuthority::CurrentHead, None) => current_ready_head_matches(connection),
        (
            PublicationAuthority::MigrationRebuild {
                contract_id,
                barrier_digest,
                attempt_id,
                attempt_count,
            },
            Some(barrier),
        ) => {
            if barrier.identity_digest() != barrier_digest
                || !migration_rebuild_barrier_token_matches(connection, barrier)?
            {
                return Ok(false);
            }
            exact_running_migration_attempt(
                connection,
                &contract_id,
                &barrier_digest,
                &attempt_id,
                attempt_count,
            )
        }
        (
            PublicationAuthority::ArtifactRepair {
                key,
                attempt_id,
                attempt_count,
            },
            None,
        ) => exact_running_artifact_attempt(connection, &key, &attempt_id, attempt_count),
        _ => Ok(false),
    }
}

pub(super) fn read_authority(
    connection: &Connection,
    generation: &str,
) -> Result<Option<PublicationAuthority>> {
    let row = connection
        .query_row(
            "SELECT authority_kind, authority_contract_id, authority_barrier_digest,
                    authority_repair_generation, authority_repair_fingerprint,
                    authority_repair_visible_epoch, authority_attempt_id,
                    authority_attempt_count
             FROM search_publication_journal WHERE generation = ?1",
            params![generation],
            |row| {
                Ok(PersistedAuthorityRow {
                    kind: row.get(0)?,
                    contract_id: row.get(1)?,
                    barrier_digest: row.get(2)?,
                    repair_generation: row.get(3)?,
                    repair_fingerprint: row.get(4)?,
                    repair_visible_epoch: row.get(5)?,
                    attempt_id: row.get(6)?,
                    attempt_count: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    row.map(parse_authority).transpose()
}

pub(crate) fn validate_persisted_authority(
    connection: &Connection,
    generation: &str,
) -> Result<()> {
    read_authority(connection, generation)?
        .map(|_| ())
        .ok_or_else(MetaStoreError::storage_invariant)
}

fn parse_authority(row: PersistedAuthorityRow) -> Result<PublicationAuthority> {
    let PersistedAuthorityRow {
        kind,
        contract_id,
        barrier_digest,
        repair_generation,
        repair_fingerprint,
        repair_visible_epoch,
        attempt_id,
        attempt_count,
    } = row;
    match (
        kind,
        contract_id,
        barrier_digest,
        repair_generation,
        repair_fingerprint,
        repair_visible_epoch,
        attempt_id,
        attempt_count,
    ) {
        (kind, None, None, None, None, None, None, None) if kind == "current_head" => {
            Ok(PublicationAuthority::CurrentHead)
        }
        (kind, Some(contract), Some(barrier), None, None, None, Some(attempt), Some(count))
            if kind == "migration_rebuild" =>
        {
            Ok(PublicationAuthority::MigrationRebuild {
                contract_id: contract.parse()?,
                barrier_digest: parse_digest(&barrier)?,
                attempt_id: parse_digest(&attempt)?,
                attempt_count: u8::try_from(count)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
            })
        }
        (
            kind,
            None,
            None,
            Some(generation),
            Some(fingerprint),
            Some(epoch),
            Some(attempt),
            Some(count),
        ) if kind == "artifact_repair" => Ok(PublicationAuthority::ArtifactRepair {
            key: ArtifactRepairKey::new(
                generation,
                parse_digest(&fingerprint)?,
                u64::try_from(epoch).map_err(|_| MetaStoreError::storage_invariant())?,
            ),
            attempt_id: parse_digest(&attempt)?,
            attempt_count: u8::try_from(count).map_err(|_| MetaStoreError::storage_invariant())?,
        }),
        _ => Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        )),
    }
}

pub(super) fn current_ready_head_matches(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM search_projection_state
                 WHERE state_key = 'default' AND service_state = 'ready'
                   AND repair_reason IS NULL AND generation IS NOT NULL
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_running_migration_attempt(
    connection: &Connection,
    contract_id: &ImportProcessingContractId,
    barrier_digest: &ContentDigest,
    attempt_id: &ContentDigest,
    attempt_count: u8,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM migration_rebuild_publication_attempt
                 WHERE state_key = 'default' AND processing_contract_id = ?1
                   AND barrier_digest = ?2 AND attempt_id = ?3
                   AND attempt_count = ?4 AND phase = 'running'
             )",
            params![
                contract_id.as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_running_artifact_attempt(
    connection: &Connection,
    key: &ArtifactRepairKey,
    attempt_id: &ContentDigest,
    attempt_count: u8,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM artifact_repair_attempt AS attempt
                 JOIN artifact_repair_context AS context
                   ON context.state_key = attempt.state_key
                  AND context.generation = attempt.generation
                  AND context.publication_fingerprint = attempt.publication_fingerprint
                  AND context.visible_epoch = attempt.visible_epoch
                 JOIN search_projection_state AS head
                   ON head.state_key = context.state_key
                  AND head.generation = context.generation
                  AND head.visible_epoch = context.visible_epoch
                 WHERE attempt.state_key = 'default' AND attempt.generation = ?1
                   AND attempt.publication_fingerprint = ?2
                   AND attempt.visible_epoch = ?3 AND attempt.attempt_id = ?4
                   AND attempt.attempt_count = ?5 AND attempt.phase = 'running'
                   AND head.service_state = 'repairing'
                   AND head.repair_reason = 'artifact_unavailable'
             )",
            params![
                key.generation(),
                key.publication_fingerprint().as_str(),
                i64::try_from(key.visible_epoch())
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_blocked_current_head(
    connection: &Connection,
    generation: Option<&str>,
    visible_epoch: u64,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM search_projection_state
                 WHERE state_key = 'default' AND service_state = 'repair_blocked'
                   AND repair_reason = 'runtime_invariant' AND generation IS ?1
                   AND visible_epoch = ?2
             )",
            params![
                generation,
                i64::try_from(visible_epoch).map_err(|_| MetaStoreError::storage_invariant())?,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_terminal_migration_cleanup_authority(
    connection: &Connection,
    contract_id: &ImportProcessingContractId,
    barrier_digest: &ContentDigest,
    attempt_id: &ContentDigest,
    attempt_count: u8,
) -> Result<bool> {
    if !migration_rebuild_terminal_block_digest_matches(connection, contract_id, barrier_digest)? {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM migration_rebuild_publication_attempt
                 WHERE state_key = 'default' AND processing_contract_id = ?1
                   AND barrier_digest = ?2 AND attempt_id = ?3
                   AND attempt_count = ?4 AND phase = 'terminal'
                   AND next_retry_at_seconds IS NULL AND last_error_class = 'cleanup'
             )",
            params![
                contract_id.as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

pub(super) fn exact_terminal_artifact_cleanup_authority(
    connection: &Connection,
    key: &ArtifactRepairKey,
    attempt_id: &ContentDigest,
    attempt_count: u8,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM artifact_repair_attempt AS attempt
                 JOIN artifact_repair_context AS context
                   ON context.state_key = attempt.state_key
                  AND context.generation = attempt.generation
                  AND context.publication_fingerprint = attempt.publication_fingerprint
                  AND context.visible_epoch = attempt.visible_epoch
                 JOIN search_projection_state AS head
                   ON head.state_key = context.state_key
                  AND head.generation = context.generation
                  AND head.visible_epoch = context.visible_epoch
                 WHERE attempt.state_key = 'default' AND attempt.generation = ?1
                   AND attempt.publication_fingerprint = ?2
                   AND attempt.visible_epoch = ?3 AND attempt.attempt_id = ?4
                   AND attempt.attempt_count = ?5 AND attempt.phase = 'terminal'
                   AND attempt.next_retry_at_seconds IS NULL
                   AND attempt.last_error_kind = 'cleanup'
                   AND head.service_state = 'repair_blocked'
                   AND head.repair_reason = 'runtime_invariant'
             )",
            params![
                key.generation(),
                key.publication_fingerprint().as_str(),
                i64::try_from(key.visible_epoch())
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                attempt_id.as_str(),
                i64::from(attempt_count),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matched| matched == 1)
        .map_err(MetaStoreError::storage)
}

fn parse_digest(value: &str) -> Result<ContentDigest> {
    ContentDigest::from_str(value).map_err(|_| MetaStoreError::storage_invariant())
}
