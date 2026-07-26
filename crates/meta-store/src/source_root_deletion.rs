use std::str::FromStr;

use core_domain::DocumentId;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::{
    MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess, Result,
    SourceRootId, UnixTimestamp,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRootDeletionPhase {
    Requested,
    Quiescing,
    Publishing,
    Purging,
    Verifying,
    Complete,
    Failed,
}

impl SourceRootDeletionPhase {
    fn storage(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Quiescing => "quiescing",
            Self::Publishing => "publishing",
            Self::Purging => "purging",
            Self::Verifying => "verifying",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "requested" => Ok(Self::Requested),
            "quiescing" => Ok(Self::Quiescing),
            "publishing" => Ok(Self::Publishing),
            "purging" => Ok(Self::Purging),
            "verifying" => Ok(Self::Verifying),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            _ => Err(MetaStoreError::invalid_value("source_root_deletion.phase")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRootDeletion {
    pub root_id: SourceRootId,
    pub phase: SourceRootDeletionPhase,
    pub affected_documents: u64,
    pub removed_documents: u64,
    pub started_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
    pub completed_at: Option<UnixTimestamp>,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn source_root_deletion(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Option<SourceRootDeletion>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT root_id, phase, affected_documents, removed_documents,
                        started_at_seconds, updated_at_seconds, completed_at_seconds
                 FROM source_root_deletion WHERE root_id = ?1",
                params![root_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(
                |(root_id, phase, affected, removed, started, updated, completed)| {
                    Ok(SourceRootDeletion {
                        root_id: SourceRootId::from_str(&root_id)?,
                        phase: SourceRootDeletionPhase::parse(&phase)?,
                        affected_documents: to_u64(affected)?,
                        removed_documents: to_u64(removed)?,
                        started_at: UnixTimestamp::from_unix_seconds(started),
                        updated_at: UnixTimestamp::from_unix_seconds(updated),
                        completed_at: completed.map(UnixTimestamp::from_unix_seconds),
                    })
                },
            )
            .transpose()
    }

    pub fn incomplete_source_root_deletions(&self) -> Result<Vec<SourceRootDeletion>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT root_id FROM source_root_deletion
                 WHERE phase NOT IN ('complete', 'failed')
                 ORDER BY started_at_seconds, root_id",
            )
            .map_err(MetaStoreError::storage)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        ids.into_iter()
            .map(|id| {
                let id = SourceRootId::from_str(&id)?;
                self.source_root_deletion(&id)?
                    .ok_or_else(MetaStoreError::storage_invariant)
            })
            .collect()
    }

    pub fn source_root_deletion_in_progress(&self, root_id: &SourceRootId) -> Result<bool> {
        Ok(self.source_root_deletion(root_id)?.is_some_and(|receipt| {
            !matches!(
                receipt.phase,
                SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
            )
        }))
    }

    pub fn begin_source_root_deletion(
        &self,
        root_id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let root_exists = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM source_root WHERE id = ?1)",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            != 0;
        if !root_exists {
            return Err(MetaStoreError::not_found("source_root"));
        }
        let documents = {
            let mut statement = transaction
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
                    DocumentId::from_str(&id)
                        .map_err(|_| MetaStoreError::invalid_value("document.id"))
                })
                .collect::<Result<Vec<_>>>()?;
            documents
        };
        transaction
            .execute(
                "INSERT INTO source_root_deletion (
                    root_id, canonical_path, phase,
                    affected_documents, removed_documents,
                    started_at_seconds, updated_at_seconds
                 )
                 SELECT id, canonical_path, 'requested', ?2, 0, ?3, ?3
                 FROM source_root
                 WHERE id = ?1
                 ON CONFLICT(root_id) DO UPDATE SET
                    canonical_path = excluded.canonical_path,
                    phase = 'requested',
                    affected_documents = excluded.affected_documents,
                    removed_documents = 0,
                    started_at_seconds = excluded.started_at_seconds,
                    updated_at_seconds = excluded.updated_at_seconds,
                    completed_at_seconds = NULL",
                params![
                    root_id.as_str(),
                    i64::try_from(documents.len())
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM source_root_deletion_document WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        for document_id in &documents {
            transaction
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
        transaction.commit().map_err(MetaStoreError::storage)?;
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn set_source_root_deletion_phase(
        &self,
        root_id: &SourceRootId,
        phase: SourceRootDeletionPhase,
        now: UnixTimestamp,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let current = self
            .source_root_deletion(root_id)?
            .ok_or_else(|| MetaStoreError::not_found("source_root_deletion"))?
            .phase;
        if !deletion_phase_transition_allowed(current, phase) {
            return Err(MetaStoreError::invalid_transition());
        }
        let completed =
            (phase == SourceRootDeletionPhase::Complete).then_some(now.as_unix_seconds());
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE source_root_deletion
                 SET phase = ?2, updated_at_seconds = MAX(updated_at_seconds, ?3),
                     completed_at_seconds = ?4
                 WHERE root_id = ?1 AND phase = ?5",
                params![
                    root_id.as_str(),
                    phase.storage(),
                    now.as_unix_seconds(),
                    completed,
                    current.storage()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::not_found("source_root_deletion"));
        }
        Ok(())
    }

    pub fn source_root_unreferenced_content_hashes(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Vec<String>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT doomed.content_hash
                 FROM source_root_deletion_document AS doomed
                 WHERE doomed.root_id = ?1
                   AND NOT EXISTS (
                     SELECT 1
                     FROM source_revision AS retained_revision
                     JOIN document AS retained
                       ON retained.id = retained_revision.document_id
                     WHERE retained_revision.document_id NOT IN (
                         SELECT document_id FROM source_root_deletion_document
                         WHERE root_id = ?1
                     )
                       AND retained.is_deleted = 0
                       AND retained.status <> 'deleted'
                       AND retained_revision.content_hash = doomed.content_hash
                   )
                 ORDER BY doomed.content_hash",
            )
            .map_err(MetaStoreError::storage)?;
        let hashes = statement
            .query_map(params![root_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage);
        hashes
    }

    pub fn purge_source_root_data(&self, root_id: &SourceRootId, now: UnixTimestamp) -> Result<u64>
    where
        Access: MetadataStoreWriteAccess,
    {
        let documents = self.source_root_deletion_document_ids(root_id)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        for document_id in &documents {
            transaction
                .execute(
                    "UPDATE document
                     SET is_deleted = 1, status = 'deleted',
                         updated_at_seconds = MAX(updated_at_seconds, ?2)
                     WHERE id = ?1",
                    params![document_id.as_str(), now.as_unix_seconds()],
                )
                .map_err(MetaStoreError::storage)?;
        }
        transaction
            .execute(
                "DELETE FROM source_occurrence WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM scan_snapshot WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM import_task
                 WHERE root_path = (
                    SELECT canonical_path FROM source_root WHERE id = ?1
                 )",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM authorized_import_root
                 WHERE canonical_root_path = (
                    SELECT canonical_path FROM source_root WHERE id = ?1
                 )",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        let receipt_updated = transaction
            .execute(
                "UPDATE source_root_deletion
                 SET removed_documents = ?2, phase = 'verifying',
                     updated_at_seconds = MAX(updated_at_seconds, ?3)
                 WHERE root_id = ?1 AND phase = 'purging'",
                params![
                    root_id.as_str(),
                    i64::try_from(documents.len())
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if receipt_updated != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        u64::try_from(documents.len()).map_err(|_| MetaStoreError::storage_invariant())
    }

    pub fn source_root_deletion_residual_count(&self, root_id: &SourceRootId) -> Result<u64> {
        let residuals = self
            .connection
            .borrow()
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM source_occurrence WHERE root_id = ?1)
                    + (SELECT COUNT(*) FROM scan_snapshot WHERE root_id = ?1)
                    + (SELECT COUNT(*) FROM pdf_reprocess_job WHERE root_id = ?1)
                    + (
                        SELECT COUNT(*) FROM import_task
                        WHERE root_path = (
                            SELECT canonical_path FROM source_root_deletion
                            WHERE root_id = ?1
                        )
                    )
                    + (
                        SELECT COUNT(*) FROM authorized_import_root
                        WHERE canonical_root_path = (
                            SELECT canonical_path FROM source_root_deletion
                            WHERE root_id = ?1
                        )
                    )
                    + (
                        SELECT COUNT(*) FROM document
                        WHERE id IN (
                            SELECT document_id
                            FROM source_root_deletion_document
                            WHERE root_id = ?1
                        )
                    )",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        to_u64(residuals)
    }

    pub fn complete_source_root_deletion(
        &self,
        root_id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let residuals = transaction
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM source_occurrence WHERE root_id = ?1)
                    + (SELECT COUNT(*) FROM scan_snapshot WHERE root_id = ?1)
                    + (SELECT COUNT(*) FROM pdf_reprocess_job WHERE root_id = ?1)
                    + (
                        SELECT COUNT(*) FROM import_task
                        WHERE root_path = (
                            SELECT canonical_path FROM source_root_deletion
                            WHERE root_id = ?1
                        )
                    )
                    + (
                        SELECT COUNT(*) FROM authorized_import_root
                        WHERE canonical_root_path = (
                            SELECT canonical_path FROM source_root_deletion
                            WHERE root_id = ?1
                        )
                    )
                    + (
                        SELECT COUNT(*) FROM document
                        WHERE id IN (
                            SELECT document_id
                            FROM source_root_deletion_document
                            WHERE root_id = ?1
                        )
                    )",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        if residuals != 0 {
            return Err(MetaStoreError::storage_invariant());
        }
        let removed = transaction
            .execute(
                "DELETE FROM source_root WHERE id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        if removed != 1 {
            return Err(MetaStoreError::not_found("source_root"));
        }
        transaction
            .execute(
                "DELETE FROM source_root_deletion_document WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        let changed = transaction
            .execute(
                "UPDATE source_root_deletion
                 SET phase = 'complete',
                     updated_at_seconds = MAX(updated_at_seconds, ?2),
                     completed_at_seconds = ?2
                 WHERE root_id = ?1 AND phase = 'verifying'",
                params![root_id.as_str(), now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn source_root_deletion_document_ids(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Vec<DocumentId>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT document_id
                 FROM source_root_deletion_document
                 WHERE root_id = ?1
                 ORDER BY document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let documents = statement
            .query_map(params![root_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let id = row.map_err(MetaStoreError::storage)?;
                DocumentId::from_str(&id).map_err(|_| MetaStoreError::invalid_value("document.id"))
            })
            .collect();
        documents
    }
}

fn to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}

fn deletion_phase_transition_allowed(
    current: SourceRootDeletionPhase,
    next: SourceRootDeletionPhase,
) -> bool {
    current == next
        || matches!(
            (current, next),
            (
                SourceRootDeletionPhase::Requested,
                SourceRootDeletionPhase::Quiescing
            ) | (
                SourceRootDeletionPhase::Quiescing,
                SourceRootDeletionPhase::Publishing
            ) | (
                SourceRootDeletionPhase::Publishing,
                SourceRootDeletionPhase::Purging
            ) | (
                SourceRootDeletionPhase::Purging,
                SourceRootDeletionPhase::Verifying
            ) | (
                SourceRootDeletionPhase::Verifying,
                SourceRootDeletionPhase::Complete
            ) | (_, SourceRootDeletionPhase::Failed)
        )
}
