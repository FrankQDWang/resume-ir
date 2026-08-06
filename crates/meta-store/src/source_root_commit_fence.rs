use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{
    schema_v38, ImportTaskId, MetaStoreError, Result, SourceRootDeletionPhase, SourceRootId,
};

struct DurableScanBinding {
    task_id: Option<String>,
    scope_task_id: Option<String>,
    snapshot_id: Option<String>,
    task_root_path: Option<String>,
    scope_root_path: Option<String>,
    snapshot_root_id: Option<String>,
    snapshot_epoch: Option<i64>,
    canonical_root_path: String,
    root_epoch: i64,
    deletion_phase: Option<String>,
}

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
    task_id: &ImportTaskId,
) -> Result<()> {
    let binding = read_durable_scan_binding(connection, root_id, task_id)?;
    let expected_task_id = Some(task_id.as_str());
    let expected_root_id = Some(root_id.as_str());
    if binding.task_id.as_deref() != expected_task_id
        || binding.scope_task_id.as_deref() != expected_task_id
        || binding.snapshot_id.as_deref() != expected_task_id
        || binding.task_root_path.as_deref() != Some(binding.canonical_root_path.as_str())
        || binding.scope_root_path.as_deref() != Some(binding.canonical_root_path.as_str())
        || binding.snapshot_root_id.as_deref() != expected_root_id
    {
        return Err(MetaStoreError::invalid_transition());
    }
    validate_epoch(binding.root_epoch)?;
    let snapshot_epoch = binding
        .snapshot_epoch
        .ok_or_else(MetaStoreError::invalid_transition)?;
    validate_epoch(snapshot_epoch)?;
    if snapshot_epoch != binding.root_epoch {
        return Err(MetaStoreError::invalid_transition());
    }
    if binding
        .deletion_phase
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

fn read_durable_scan_binding(
    connection: &Connection,
    root_id: &SourceRootId,
    task_id: &ImportTaskId,
) -> Result<DurableScanBinding> {
    connection
        .query_row(
            "SELECT task.id, scope.import_task_id, snapshot.id,
                    task.root_path, scope.canonical_root_path, snapshot.root_id,
                    snapshot.root_revocation_epoch, root.canonical_path,
                    root.revocation_epoch, deletion.phase
             FROM source_root AS root
             LEFT JOIN import_task AS task ON task.id = ?2
             LEFT JOIN import_scan_scope AS scope ON scope.import_task_id = ?2
             LEFT JOIN scan_snapshot AS snapshot ON snapshot.id = ?2
             LEFT JOIN source_root_deletion AS deletion ON deletion.root_id = root.id
             WHERE root.id = ?1",
            params![root_id.as_str(), task_id.as_str()],
            |row| {
                Ok(DurableScanBinding {
                    task_id: row.get(0)?,
                    scope_task_id: row.get(1)?,
                    snapshot_id: row.get(2)?,
                    task_root_path: row.get(3)?,
                    scope_root_path: row.get(4)?,
                    snapshot_root_id: row.get(5)?,
                    snapshot_epoch: row.get(6)?,
                    canonical_root_path: row.get(7)?,
                    root_epoch: row.get(8)?,
                    deletion_phase: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::invalid_transition)
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
