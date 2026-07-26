use rusqlite::{params, TransactionBehavior};

use crate::{
    MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess,
    PrivacyPurgeReport, Result, SourceRootId, PRIVACY_PURGE_BATCH_LIMIT,
};

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Physically removes one bounded batch belonging to an exact root-deletion
    /// receipt without consuming unrelated tombstones elsewhere in the store.
    pub fn purge_source_root_deleted_documents(
        &self,
        root_id: &SourceRootId,
    ) -> Result<PrivacyPurgeReport>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        connection
            .execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS source_root_privacy_batch (
                    document_id TEXT PRIMARY KEY NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TEMP TABLE IF NOT EXISTS source_root_affected_candidate (
                    candidate_id TEXT PRIMARY KEY NOT NULL
                 ) WITHOUT ROWID;
                 DELETE FROM source_root_privacy_batch;
                 DELETE FROM source_root_affected_candidate;",
            )
            .map_err(MetaStoreError::storage)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let batch_limit = i64::try_from(PRIVACY_PURGE_BATCH_LIMIT)
            .map_err(|_| MetaStoreError::storage_invariant())?;
        transaction
            .execute(
                "INSERT INTO source_root_privacy_batch (document_id)
                 SELECT DISTINCT doomed.document_id
                 FROM source_root_deletion_document AS doomed
                 JOIN document ON document.id = doomed.document_id
                 WHERE doomed.root_id = ?1
                   AND (document.is_deleted = 1 OR document.status = 'deleted')
                 ORDER BY doomed.document_id
                 LIMIT ?2",
                params![root_id.as_str(), batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        let batch_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM source_root_privacy_batch",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        if batch_count == 0 {
            let remaining = source_root_remaining_documents(&transaction, root_id)?;
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(PrivacyPurgeReport {
                deleted_documents: 0,
                remaining_tombstones: remaining,
            });
        }
        let has_active_projection = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM active_search_projection AS projection
                    JOIN source_root_privacy_batch AS batch
                      ON batch.document_id = projection.document_id
                 )",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            != 0;
        if has_active_projection {
            return Err(MetaStoreError::invalid_transition());
        }

        transaction
            .execute_batch(
                "INSERT OR IGNORE INTO source_root_affected_candidate (candidate_id)
                 SELECT assignment.candidate_id
                 FROM resume_version_candidate AS assignment
                 JOIN resume_version AS version
                   ON version.id = assignment.resume_version_id
                 JOIN source_root_privacy_batch AS batch
                   ON batch.document_id = version.document_id;

                 INSERT OR IGNORE INTO source_root_affected_candidate (candidate_id)
                 SELECT seal.candidate_id
                 FROM resume_version_seal AS seal
                 JOIN resume_version AS version
                   ON version.id = seal.resume_version_id
                 JOIN source_root_privacy_batch AS batch
                   ON batch.document_id = version.document_id
                 WHERE seal.candidate_id IS NOT NULL;

                 INSERT OR IGNORE INTO source_root_affected_candidate (candidate_id)
                 SELECT conflict.email_candidate_id
                 FROM candidate_contact_conflict AS conflict
                 JOIN resume_version AS version
                   ON version.id = conflict.resume_version_id
                 JOIN source_root_privacy_batch AS batch
                   ON batch.document_id = version.document_id
                 UNION
                 SELECT conflict.phone_candidate_id
                 FROM candidate_contact_conflict AS conflict
                 JOIN resume_version AS version
                   ON version.id = conflict.resume_version_id
                 JOIN source_root_privacy_batch AS batch
                   ON batch.document_id = version.document_id;

                 DELETE FROM resume_version_seal
                 WHERE resume_version_id IN (
                    SELECT version.id
                    FROM resume_version AS version
                    JOIN source_root_privacy_batch AS batch
                      ON batch.document_id = version.document_id
                 );",
            )
            .map_err(MetaStoreError::storage)?;
        let deleted = transaction
            .execute(
                "DELETE FROM document
                 WHERE id IN (SELECT document_id FROM source_root_privacy_batch)",
                [],
            )
            .map_err(MetaStoreError::storage)?;
        if i64::try_from(deleted).ok() != Some(batch_count) {
            return Err(MetaStoreError::storage_invariant());
        }
        transaction
            .execute_batch(
                "UPDATE candidate
                 SET version_count = (
                    SELECT COUNT(*)
                    FROM active_search_projection AS projection
                    JOIN resume_version_candidate AS assignment
                      ON assignment.resume_version_id = projection.resume_version_id
                    WHERE assignment.candidate_id = candidate.id
                 )
                 WHERE id IN (
                    SELECT candidate_id FROM source_root_affected_candidate
                 );

                 DELETE FROM candidate
                 WHERE id IN (
                    SELECT candidate_id FROM source_root_affected_candidate
                 )
                   AND NOT EXISTS (
                    SELECT 1 FROM resume_version_candidate AS assignment
                    WHERE assignment.candidate_id = candidate.id
                   )
                   AND NOT EXISTS (
                    SELECT 1 FROM resume_version_seal AS seal
                    WHERE seal.candidate_id = candidate.id
                   )
                   AND NOT EXISTS (
                    SELECT 1 FROM candidate_contact_conflict AS conflict
                    WHERE conflict.email_candidate_id = candidate.id
                       OR conflict.phone_candidate_id = candidate.id
                   );

                 DELETE FROM source_root_privacy_batch;
                 DELETE FROM source_root_affected_candidate;",
            )
            .map_err(MetaStoreError::storage)?;
        let remaining = source_root_remaining_documents(&transaction, root_id)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(PrivacyPurgeReport {
            deleted_documents: deleted,
            remaining_tombstones: remaining,
        })
    }
}

fn source_root_remaining_documents(
    transaction: &rusqlite::Transaction<'_>,
    root_id: &SourceRootId,
) -> Result<u64> {
    let remaining = transaction
        .query_row(
            "SELECT COUNT(DISTINCT doomed.document_id)
             FROM source_root_deletion_document AS doomed
             JOIN document ON document.id = doomed.document_id
             WHERE doomed.root_id = ?1",
            params![root_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u64::try_from(remaining).map_err(|_| MetaStoreError::storage_invariant())
}
