use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{schema_v38, MetaStoreError, Result, SourceRootDeletionPhase, SourceRootId};

pub(super) fn source_root_is_deleting(
    connection: &Connection,
    canonical_root_path: &str,
) -> Result<bool> {
    let phase = connection
        .query_row(
            "SELECT deletion.phase
             FROM source_root
             JOIN source_root_deletion AS deletion
               ON deletion.root_id = source_root.id
             WHERE source_root.canonical_path = ?1",
            [canonical_root_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    phase
        .map(|value| SourceRootDeletionPhase::parse(&value).map(|phase| phase.is_active()))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(super) fn capture_scan_epoch(connection: &Connection, root_id: &SourceRootId) -> Result<i64> {
    read_epoch(connection, root_id)
}

pub(super) fn admit_scan(connection: &Connection, root_id: &SourceRootId) -> Result<i64> {
    let epoch = read_epoch(connection, root_id)?;
    if deletion_phase(connection, root_id)?.is_some_and(SourceRootDeletionPhase::is_active) {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(epoch)
}

pub(super) fn validate_scan_commit(
    connection: &Connection,
    root_id: &SourceRootId,
    captured_epoch: i64,
) -> Result<()> {
    validate_epoch(captured_epoch)?;
    if read_epoch(connection, root_id)? != captured_epoch
        || deletion_phase(connection, root_id)?.is_some_and(SourceRootDeletionPhase::is_active)
    {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(())
}

pub(super) fn bump_epoch(transaction: &Transaction<'_>, root_id: &SourceRootId) -> Result<i64> {
    let current = read_epoch(transaction, root_id)?;
    if current == schema_v38::MAX_ROOT_REVOCATION_EPOCH {
        return Err(MetaStoreError::invalid_transition());
    }
    let next = current + 1;
    let changed = transaction
        .execute(
            "UPDATE source_root
             SET revocation_epoch = ?2
             WHERE id = ?1
               AND typeof(revocation_epoch) = 'integer'
               AND revocation_epoch = ?3
               AND revocation_epoch < ?4",
            params![
                root_id.as_str(),
                next,
                current,
                schema_v38::MAX_ROOT_REVOCATION_EPOCH
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(next)
}

pub(super) fn backfill_v38(transaction: &Transaction<'_>) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT deletion.root_id, deletion.phase,
                    EXISTS(SELECT 1 FROM source_root WHERE id = deletion.root_id)
             FROM source_root_deletion AS deletion
             ORDER BY deletion.root_id",
        )
        .map_err(MetaStoreError::migration)?;
    let receipts = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
            ))
        })
        .map_err(MetaStoreError::migration)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(MetaStoreError::migration)?;
    drop(statement);
    for (root_id, phase, root_exists) in receipts {
        let phase = SourceRootDeletionPhase::parse(&phase)?;
        if root_exists == (phase == SourceRootDeletionPhase::Complete) {
            return Err(MetaStoreError::storage_invariant());
        }
        if root_exists {
            let changed = transaction
                .execute(
                    "UPDATE source_root SET revocation_epoch = 1
                     WHERE id = ?1 AND revocation_epoch = 0",
                    [root_id],
                )
                .map_err(MetaStoreError::migration)?;
            if changed != 1 {
                return Err(MetaStoreError::storage_invariant());
            }
        }
    }
    Ok(())
}

fn deletion_phase(
    connection: &Connection,
    root_id: &SourceRootId,
) -> Result<Option<SourceRootDeletionPhase>> {
    connection
        .query_row(
            "SELECT phase FROM source_root_deletion WHERE root_id = ?1",
            [root_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|value| SourceRootDeletionPhase::parse(&value))
        .transpose()
}

fn read_epoch(connection: &Connection, root_id: &SourceRootId) -> Result<i64> {
    let epoch = connection
        .query_row(
            "SELECT revocation_epoch FROM source_root WHERE id = ?1",
            [root_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
    validate_epoch(epoch)?;
    Ok(epoch)
}

fn validate_epoch(epoch: i64) -> Result<()> {
    if !(0..=schema_v38::MAX_ROOT_REVOCATION_EPOCH).contains(&epoch) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}
