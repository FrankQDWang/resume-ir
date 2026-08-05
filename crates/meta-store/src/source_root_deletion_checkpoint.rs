use std::str::FromStr;

#[cfg(test)]
use std::cell::Cell;

use core_domain::DocumentId;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::{schema_v37, MetaStoreError, Result, SourceRootId, UnixTimestamp};

pub(super) const ATTEMPT_ADMISSION_SQL: &str = "SELECT phase, checkpoint_protocol_version
     FROM source_root_deletion
     WHERE root_id = ?1";

const CURRENT_SNAPSHOT_SQL: &str = "WITH exclusive_document AS (
       SELECT DISTINCT occurrence.document_id
       FROM source_occurrence AS occurrence
       WHERE occurrence.root_id = ?1
         AND NOT EXISTS (
           SELECT 1 FROM source_occurrence AS other
           WHERE other.document_id = occurrence.document_id
             AND other.root_id <> occurrence.root_id
             AND other.state = 'present'
         )
     )
     SELECT exclusive.document_id, revision.content_hash
     FROM exclusive_document AS exclusive
     JOIN source_revision AS revision
       ON revision.document_id = exclusive.document_id
     UNION
     SELECT exclusive.document_id, document.content_hash
     FROM exclusive_document AS exclusive
     JOIN document ON document.id = exclusive.document_id
     ORDER BY 1, 2";

#[cfg(test)]
thread_local! {
    static SNAPSHOT_CENSUS_COUNT: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_snapshot_census_count() {
    SNAPSHOT_CENSUS_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn snapshot_census_count() -> u64 {
    SNAPSHOT_CENSUS_COUNT.with(Cell::get)
}

#[derive(Debug, Eq, PartialEq)]
struct SnapshotTuple {
    document_id: DocumentId,
    content_hash: String,
}

pub(super) struct CurrentSourceRootDeletionSnapshot {
    tuples: Vec<SnapshotTuple>,
}

impl CurrentSourceRootDeletionSnapshot {
    pub(super) fn read(connection: &Connection, root_id: &SourceRootId) -> Result<Self> {
        #[cfg(test)]
        SNAPSHOT_CENSUS_COUNT.with(|count| count.set(count.get().saturating_add(1)));

        let mut statement = connection
            .prepare(CURRENT_SNAPSHOT_SQL)
            .map_err(MetaStoreError::storage)?;
        let tuples = statement
            .query_map(params![root_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let (document_id, content_hash) = row.map_err(MetaStoreError::storage)?;
                Ok(SnapshotTuple {
                    document_id: DocumentId::from_str(&document_id)
                        .map_err(|_| MetaStoreError::invalid_value("document.id"))?,
                    content_hash,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { tuples })
    }

    pub(super) fn affected_documents(&self) -> Result<i64> {
        let distinct_document_count = self
            .tuples
            .iter()
            .map(|tuple| &tuple.document_id)
            .fold((0_usize, None), |(count, previous), document_id| {
                if previous == Some(document_id) {
                    (count, previous)
                } else {
                    (count.saturating_add(1), Some(document_id))
                }
            })
            .0;
        i64::try_from(distinct_document_count).map_err(|_| MetaStoreError::storage_invariant())
    }

    fn matches_persisted(&self, connection: &Connection, root_id: &SourceRootId) -> Result<bool> {
        let mut statement = connection
            .prepare(
                "SELECT document_id, content_hash
                 FROM source_root_deletion_document
                 WHERE root_id = ?1
                 ORDER BY document_id, content_hash",
            )
            .map_err(MetaStoreError::storage)?;
        let persisted = statement
            .query_map(params![root_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let (document_id, content_hash) = row.map_err(MetaStoreError::storage)?;
                Ok(SnapshotTuple {
                    document_id: DocumentId::from_str(&document_id)
                        .map_err(|_| MetaStoreError::invalid_value("document.id"))?,
                    content_hash,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(self.tuples == persisted)
    }

    pub(super) fn replace(&self, connection: &Connection, root_id: &SourceRootId) -> Result<()> {
        connection
            .execute(
                "DELETE FROM source_root_deletion_document WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        for tuple in &self.tuples {
            connection
                .execute(
                    "INSERT INTO source_root_deletion_document (
                        root_id, document_id, content_hash
                     ) VALUES (?1, ?2, ?3)",
                    params![
                        root_id.as_str(),
                        tuple.document_id.as_str(),
                        tuple.content_hash
                    ],
                )
                .map_err(MetaStoreError::storage)?;
        }
        Ok(())
    }
}

struct LegacyQuiescingReconciliation {
    canonical_path: String,
    snapshot: CurrentSourceRootDeletionSnapshot,
    snapshot_changed: bool,
}

pub(super) fn begin_attempt(
    connection: &mut Connection,
    root_id: &SourceRootId,
    now: UnixTimestamp,
) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    let (phase, checkpoint_protocol_version) = transaction
        .query_row(ATTEMPT_ADMISSION_SQL, params![root_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::invalid_transition)?;
    if matches!(phase.as_str(), "complete" | "failed") {
        return Err(MetaStoreError::invalid_transition());
    }
    if !matches!(
        checkpoint_protocol_version,
        schema_v37::LEGACY_OR_UNATTESTED | schema_v37::SNAPSHOT_INVARIANT_V2
    ) {
        return Err(MetaStoreError::storage_invariant());
    }

    let reconciliation = if phase == "quiescing"
        && checkpoint_protocol_version == schema_v37::LEGACY_OR_UNATTESTED
    {
        Some(prepare_legacy_quiescing_reconciliation(
            &transaction,
            root_id,
        )?)
    } else {
        None
    };

    update_attempt_evidence(&transaction, root_id, now)?;
    if let Some(reconciliation) = reconciliation {
        if reconciliation.snapshot_changed {
            reconciliation.snapshot.replace(&transaction, root_id)?;
        }
        let changed = transaction
            .execute(
                "UPDATE source_root_deletion
                 SET canonical_path = ?2,
                     affected_documents = ?3,
                     checkpoint_protocol_version = ?4,
                     updated_at_seconds = MAX(updated_at_seconds, ?5)
                 WHERE root_id = ?1
                   AND phase = 'quiescing'
                   AND checkpoint_protocol_version = ?6
                   AND removed_documents = 0
                   AND completed_at_seconds IS NULL",
                params![
                    root_id.as_str(),
                    reconciliation.canonical_path,
                    reconciliation.snapshot.affected_documents()?,
                    schema_v37::SNAPSHOT_INVARIANT_V2,
                    now.as_unix_seconds(),
                    schema_v37::LEGACY_OR_UNATTESTED,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
    }
    transaction.commit().map_err(MetaStoreError::storage)
}

fn prepare_legacy_quiescing_reconciliation(
    transaction: &Transaction<'_>,
    root_id: &SourceRootId,
) -> Result<LegacyQuiescingReconciliation> {
    let preconditions = transaction
        .query_row(
            "SELECT root.canonical_path,
                    deletion.removed_documents,
                    deletion.completed_at_seconds,
                    (SELECT COUNT(*)
                     FROM source_root_deletion_attempt_evidence AS evidence
                     WHERE evidence.root_id = deletion.root_id)
             FROM source_root_deletion AS deletion
             LEFT JOIN source_root AS root ON root.id = deletion.root_id
             WHERE deletion.root_id = ?1
               AND deletion.phase = 'quiescing'
               AND deletion.checkpoint_protocol_version = ?2",
            params![root_id.as_str(), schema_v37::LEGACY_OR_UNATTESTED],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::invalid_transition)?;
    let (canonical_path, removed_documents, completed_at_seconds, evidence_rows) = preconditions;
    let canonical_path = canonical_path.ok_or_else(MetaStoreError::storage_invariant)?;
    if removed_documents != 0 || completed_at_seconds.is_some() {
        return Err(MetaStoreError::invalid_transition());
    }
    if evidence_rows != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    let snapshot = CurrentSourceRootDeletionSnapshot::read(transaction, root_id)?;
    let snapshot_changed = !snapshot.matches_persisted(transaction, root_id)?;
    Ok(LegacyQuiescingReconciliation {
        canonical_path,
        snapshot,
        snapshot_changed,
    })
}

fn update_attempt_evidence(
    transaction: &Transaction<'_>,
    root_id: &SourceRootId,
    now: UnixTimestamp,
) -> Result<()> {
    let changed = transaction
        .execute(
            "UPDATE source_root_deletion_attempt_evidence
             SET attempt_count = CASE
                   WHEN attempt_count < 9007199254740991
                   THEN attempt_count + 1
                   ELSE attempt_count
                 END,
                 last_attempt_at_seconds = MAX(
                    COALESCE(last_attempt_at_seconds, 0), ?2
                 ),
                 last_error_phase = NULL,
                 last_error_code = NULL,
                 last_error_at_seconds = NULL
             WHERE root_id = ?1
               AND EXISTS (
                 SELECT 1 FROM source_root_deletion AS deletion
                 WHERE deletion.root_id = ?1
                   AND deletion.phase NOT IN ('complete', 'failed')
               )",
            params![root_id.as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(())
}

pub(super) fn advance_requested_to_quiescing(
    connection: &mut Connection,
    root_id: &SourceRootId,
    now: UnixTimestamp,
) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    let canonical_path = transaction
        .query_row(
            "SELECT root.canonical_path
             FROM source_root_deletion AS deletion
             JOIN source_root AS root ON root.id = deletion.root_id
             WHERE deletion.root_id = ?1
               AND deletion.phase = 'requested'
               AND deletion.removed_documents = 0
               AND deletion.completed_at_seconds IS NULL",
            params![root_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(MetaStoreError::invalid_transition)?;
    let snapshot = CurrentSourceRootDeletionSnapshot::read(&transaction, root_id)?;
    snapshot.replace(&transaction, root_id)?;
    let changed = transaction
        .execute(
            "UPDATE source_root_deletion
             SET canonical_path = ?2,
                 affected_documents = ?3,
                 phase = 'quiescing',
                 checkpoint_protocol_version = ?4,
                 updated_at_seconds = MAX(updated_at_seconds, ?5)
             WHERE root_id = ?1
               AND phase = 'requested'
               AND removed_documents = 0
               AND completed_at_seconds IS NULL",
            params![
                root_id.as_str(),
                canonical_path,
                snapshot.affected_documents()?,
                schema_v37::SNAPSHOT_INVARIANT_V2,
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::invalid_transition());
    }
    transaction.commit().map_err(MetaStoreError::storage)
}
