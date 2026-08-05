use std::str::FromStr;

use core_domain::DocumentId;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{MetaStoreError, Result, SourceRootId, UnixTimestamp};

pub(super) struct CurrentSourceRootDeletionSnapshot {
    documents: Vec<DocumentId>,
}

impl CurrentSourceRootDeletionSnapshot {
    pub(super) fn read(connection: &Connection, root_id: &SourceRootId) -> Result<Self> {
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT occurrence.document_id
                 FROM source_occurrence AS occurrence
                 WHERE occurrence.root_id = ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM source_occurrence AS other
                     WHERE other.document_id = occurrence.document_id
                       AND other.root_id <> occurrence.root_id
                       AND other.state = 'present'
                   )
                 ORDER BY occurrence.document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let documents = statement
            .query_map(params![root_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let id = row.map_err(MetaStoreError::storage)?;
                DocumentId::from_str(&id).map_err(|_| MetaStoreError::invalid_value("document.id"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { documents })
    }

    pub(super) fn affected_documents(&self) -> Result<i64> {
        i64::try_from(self.documents.len()).map_err(|_| MetaStoreError::storage_invariant())
    }

    pub(super) fn replace(&self, connection: &Connection, root_id: &SourceRootId) -> Result<()> {
        connection
            .execute(
                "DELETE FROM source_root_deletion_document WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        for document_id in &self.documents {
            connection
                .execute(
                    "INSERT INTO source_root_deletion_document (
                        root_id, document_id, content_hash
                     )
                     SELECT ?1, revision.document_id, revision.content_hash
                     FROM source_revision AS revision
                     WHERE revision.document_id = ?2
                     UNION
                     SELECT ?1, document.id, document.content_hash
                     FROM document
                     WHERE document.id = ?2",
                    params![root_id.as_str(), document_id.as_str()],
                )
                .map_err(MetaStoreError::storage)?;
        }
        Ok(())
    }
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
                 updated_at_seconds = MAX(updated_at_seconds, ?4)
             WHERE root_id = ?1
               AND phase = 'requested'
               AND removed_documents = 0
               AND completed_at_seconds IS NULL",
            params![
                root_id.as_str(),
                canonical_path,
                snapshot.affected_documents()?,
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::invalid_transition());
    }
    transaction.commit().map_err(MetaStoreError::storage)
}
