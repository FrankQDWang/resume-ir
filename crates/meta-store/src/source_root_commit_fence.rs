use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{
    schema_v38, ImportTaskId, MetaStoreError, Result, SourceRootDeletionPhase, SourceRootId,
};

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
    task_id: &ImportTaskId,
) -> Result<()> {
    let deletion_phase = connection
        .query_row(
            "SELECT deletion.phase
             FROM source_root AS root
             JOIN import_task AS task
               ON task.id = ?2 AND task.root_path = root.canonical_path
             JOIN import_scan_scope AS scope
               ON scope.import_task_id = task.id
              AND scope.canonical_root_path = root.canonical_path
             JOIN scan_snapshot AS snapshot
               ON snapshot.id = task.id
              AND snapshot.root_id = root.id
              AND typeof(snapshot.root_revocation_epoch) = 'integer'
              AND snapshot.root_revocation_epoch = root.revocation_epoch
             LEFT JOIN source_root_deletion AS deletion ON deletion.root_id = root.id
             WHERE root.id = ?1
               AND typeof(root.revocation_epoch) = 'integer'
               AND root.revocation_epoch BETWEEN 0 AND ?3",
            params![
                root_id.as_str(),
                task_id.as_str(),
                schema_v38::MAX_ROOT_REVOCATION_EPOCH
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::invalid_transition)?;
    if deletion_phase
        .map(|value| SourceRootDeletionPhase::parse(&value))
        .transpose()?
        .is_some_and(SourceRootDeletionPhase::is_active)
    {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(())
}

pub(super) fn bump_epoch(transaction: &Transaction<'_>, root_id: &SourceRootId) -> Result<i64> {
    deletion_phase(transaction, root_id)?;
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
    validate_receipt_root_invariants(transaction)?;
    let expected = transaction
        .query_row(
            "SELECT COUNT(*) FROM source_root
             WHERE EXISTS (
                SELECT 1 FROM source_root_deletion
                WHERE source_root_deletion.root_id = source_root.id
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::migration)?;
    let changed = transaction
        .execute(
            "UPDATE source_root SET revocation_epoch = 1
             WHERE revocation_epoch = 0
               AND EXISTS (
                  SELECT 1 FROM source_root_deletion
                  WHERE source_root_deletion.root_id = source_root.id
               )",
            [],
        )
        .map_err(MetaStoreError::migration)?;
    if i64::try_from(changed) != Ok(expected) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn validate_receipt_root_invariants(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT deletion.phase,
                    EXISTS(SELECT 1 FROM source_root WHERE id = deletion.root_id)
             FROM source_root_deletion AS deletion
             ORDER BY deletion.root_id",
        )
        .map_err(MetaStoreError::migration)?;
    let mut receipts = statement.query([]).map_err(MetaStoreError::migration)?;
    while let Some(row) = receipts.next().map_err(MetaStoreError::migration)? {
        let phase = row.get::<_, String>(0).map_err(MetaStoreError::migration)?;
        let root_exists = row.get::<_, i64>(1).map_err(MetaStoreError::migration)? != 0;
        let phase = SourceRootDeletionPhase::parse(&phase)?;
        if root_exists == (phase == SourceRootDeletionPhase::Complete) {
            return Err(MetaStoreError::storage_invariant());
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

pub(super) fn read_epoch(connection: &Connection, root_id: &SourceRootId) -> Result<i64> {
    let epoch = connection
        .query_row(
            "SELECT revocation_epoch FROM source_root WHERE id = ?1",
            [root_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::not_found("source_root"))?;
    if !(0..=schema_v38::MAX_ROOT_REVOCATION_EPOCH).contains(&epoch) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(epoch)
}
