use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess, Result};

pub const PRIVACY_PURGE_BATCH_LIMIT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyPurgeReport {
    pub deleted_documents: usize,
    pub remaining_tombstones: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivacyMaintenanceFailpoint {
    None,
    #[cfg(test)]
    AfterDeleteCommit,
    #[cfg(test)]
    AfterVacuum,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Physically purges at most one bounded batch, then vacuums the rollback
    /// journal store before clearing the durable privacy-maintenance receipt.
    pub fn purge_deleted_documents(&self) -> Result<PrivacyPurgeReport>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.purge_deleted_documents_inner(PrivacyMaintenanceFailpoint::None)
    }

    fn purge_deleted_documents_inner(
        &self,
        failpoint: PrivacyMaintenanceFailpoint,
    ) -> Result<PrivacyPurgeReport>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        complete_pending_privacy_compaction(&connection, self.file_backed, failpoint)?;
        connection
            .execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS privacy_affected_candidate (
                    candidate_id TEXT PRIMARY KEY NOT NULL
                 ) WITHOUT ROWID;
                 DELETE FROM privacy_affected_candidate;",
            )
            .map_err(MetaStoreError::storage)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let batch_limit = i64::try_from(PRIVACY_PURGE_BATCH_LIMIT)
            .map_err(|_| MetaStoreError::storage_invariant())?;
        let batch_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 )",
                params![batch_limit],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        if batch_count == 0 {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(PrivacyPurgeReport {
                deleted_documents: 0,
                remaining_tombstones: 0,
            });
        }
        let has_active_projection = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM active_search_projection AS projection
                    JOIN (
                        SELECT id FROM document
                        WHERE is_deleted = 1 OR status = 'deleted'
                        ORDER BY id
                        LIMIT ?1
                    ) AS batch ON batch.id = projection.document_id
                 )",
                params![batch_limit],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            != 0;
        if has_active_projection {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO privacy_affected_candidate (candidate_id)
                 SELECT assignment.candidate_id
                 FROM resume_version_candidate AS assignment
                 JOIN resume_version AS version
                   ON version.id = assignment.resume_version_id
                 JOIN (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 ) AS batch ON batch.id = version.document_id",
                params![batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO privacy_affected_candidate (candidate_id)
                 SELECT seal.candidate_id
                 FROM resume_version_seal AS seal
                 JOIN resume_version AS version
                   ON version.id = seal.resume_version_id
                 JOIN (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 ) AS batch ON batch.id = version.document_id
                 WHERE seal.candidate_id IS NOT NULL",
                params![batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO privacy_affected_candidate (candidate_id)
                 SELECT conflict.email_candidate_id
                 FROM candidate_contact_conflict AS conflict
                 JOIN resume_version AS version
                   ON version.id = conflict.resume_version_id
                 JOIN (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 ) AS batch ON batch.id = version.document_id
                 UNION
                 SELECT conflict.phone_candidate_id
                 FROM candidate_contact_conflict AS conflict
                 JOIN resume_version AS version
                   ON version.id = conflict.resume_version_id
                 JOIN (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 ) AS batch ON batch.id = version.document_id",
                params![batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM resume_version_seal
                 WHERE resume_version_id IN (
                    SELECT version.id
                    FROM resume_version AS version
                    JOIN (
                        SELECT id FROM document
                        WHERE is_deleted = 1 OR status = 'deleted'
                        ORDER BY id
                        LIMIT ?1
                    ) AS batch ON batch.id = version.document_id
                 )",
                params![batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        let deleted = transaction
            .execute(
                "DELETE FROM document
                 WHERE id IN (
                    SELECT id FROM document
                    WHERE is_deleted = 1 OR status = 'deleted'
                    ORDER BY id
                    LIMIT ?1
                 )",
                params![batch_limit],
            )
            .map_err(MetaStoreError::storage)?;
        if i64::try_from(deleted).ok() != Some(batch_count) {
            return Err(MetaStoreError::storage_invariant());
        }
        transaction
            .execute(
                "UPDATE candidate
                 SET version_count = (
                    SELECT COUNT(*)
                    FROM active_search_projection AS projection
                    JOIN resume_version_candidate AS assignment
                      ON assignment.resume_version_id = projection.resume_version_id
                    WHERE assignment.candidate_id = candidate.id
                 )
                 WHERE id IN (SELECT candidate_id FROM privacy_affected_candidate)",
                [],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM candidate
                 WHERE id IN (SELECT candidate_id FROM privacy_affected_candidate)
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
                   )",
                [],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute("DELETE FROM privacy_affected_candidate", [])
            .map_err(MetaStoreError::storage)?;
        let receipt_changed = transaction
            .execute(
                "UPDATE privacy_maintenance_state
                 SET compaction_pending = 1
                 WHERE state_key = 'default'",
                [],
            )
            .map_err(MetaStoreError::storage)?;
        if receipt_changed != 1 {
            return Err(MetaStoreError::storage_invariant());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;

        #[cfg(test)]
        if failpoint == PrivacyMaintenanceFailpoint::AfterDeleteCommit {
            return Err(MetaStoreError::storage_invariant());
        }
        complete_pending_privacy_compaction(&connection, self.file_backed, failpoint)?;
        let remaining = connection
            .query_row(
                "SELECT COUNT(*) FROM document
                 WHERE is_deleted = 1 OR status = 'deleted'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        Ok(PrivacyPurgeReport {
            deleted_documents: deleted,
            remaining_tombstones: u64::try_from(remaining)
                .map_err(|_| MetaStoreError::storage_invariant())?,
        })
    }
}

pub(super) fn configure_privacy_maintenance(
    connection: &Connection,
    file_backed: bool,
) -> Result<()> {
    connection
        .execute_batch("PRAGMA secure_delete = ON;")
        .map_err(MetaStoreError::storage)?;
    let secure_delete = connection
        .query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0))
        .map_err(MetaStoreError::storage)?;
    if secure_delete != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    complete_pending_privacy_compaction(connection, file_backed, PrivacyMaintenanceFailpoint::None)
}

pub(super) fn complete_privacy_maintenance_after_migration(
    connection: &Connection,
    file_backed: bool,
) -> Result<()> {
    complete_pending_privacy_compaction(connection, file_backed, PrivacyMaintenanceFailpoint::None)
}

fn complete_pending_privacy_compaction(
    connection: &Connection,
    file_backed: bool,
    _failpoint: PrivacyMaintenanceFailpoint,
) -> Result<()> {
    let table_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'privacy_maintenance_state'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .is_some();
    if !table_exists {
        return Ok(());
    }
    let pending = connection
        .query_row(
            "SELECT compaction_pending FROM privacy_maintenance_state
             WHERE state_key = 'default'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if pending != 1 {
        if pending == 0 {
            return Ok(());
        }
        return Err(MetaStoreError::storage_invariant());
    }

    if file_backed {
        connection
            .execute_batch("VACUUM;")
            .map_err(MetaStoreError::storage)?;
        #[cfg(test)]
        if _failpoint == PrivacyMaintenanceFailpoint::AfterVacuum {
            return Err(MetaStoreError::storage_invariant());
        }
    }

    let changed = connection
        .execute(
            "UPDATE privacy_maintenance_state
             SET compaction_pending = 0
             WHERE state_key = 'default' AND compaction_pending = 1",
            [],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

#[cfg(test)]
#[path = "privacy_maintenance_tests.rs"]
mod tests;
