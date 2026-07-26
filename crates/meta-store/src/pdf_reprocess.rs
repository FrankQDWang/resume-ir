use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::str::FromStr;

use crate::{
    ImportTaskId, MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess,
    Result, SourceRoot, SourceRootId, UnixTimestamp,
};

const MAX_REPROCESS_ATTEMPTS: i64 = 16;

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn next_pdf_reprocess_root(&self, parser_contract: &str) -> Result<Option<SourceRoot>> {
        if parser_contract.is_empty() || parser_contract.len() > 64 {
            return Err(MetaStoreError::invalid_value(
                "pdf_reprocess_job.parser_contract",
            ));
        }
        let root_id = self
            .connection
            .borrow()
            .query_row(
                "SELECT job.root_id
                 FROM pdf_reprocess_job AS job
                 JOIN source_root AS root ON root.id = job.root_id
                 WHERE job.state = 'queued'
                   AND job.attempts < ?1
                   AND job.parser_contract = ?2
                   AND root.state = 'active'
                   AND NOT EXISTS (
                     SELECT 1 FROM source_root_deletion AS deletion
                     WHERE deletion.root_id = job.root_id
                       AND deletion.phase NOT IN ('complete', 'failed')
                   )
                 ORDER BY job.queued_at_seconds, job.root_id
                 LIMIT 1",
                params![MAX_REPROCESS_ATTEMPTS, parser_contract],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        root_id
            .map(|root_id| {
                let root_id = SourceRootId::from_str(&root_id)?;
                self.source_root(&root_id)?
                    .ok_or_else(MetaStoreError::storage_invariant)
            })
            .transpose()
    }

    pub fn mark_pdf_reprocess_root_scheduled(
        &self,
        root_id: &SourceRootId,
        parser_contract: &str,
        task_id: &ImportTaskId,
        now: UnixTimestamp,
    ) -> Result<usize>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let updated = transaction
            .execute(
                "UPDATE pdf_reprocess_job
                 SET state = 'scheduled',
                     attempts = attempts + 1,
                     scheduled_task_id = ?1,
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE root_id = ?3
                   AND parser_contract = ?4
                   AND state = 'queued'
                   AND attempts < ?5
                   AND EXISTS (
                     SELECT 1 FROM import_task AS task
                     WHERE task.id = ?1
                       AND task.status IN ('queued', 'running')
                   )
                   AND EXISTS (
                     SELECT 1 FROM scan_snapshot AS snapshot
                     WHERE snapshot.id = ?1
                       AND snapshot.root_id = pdf_reprocess_job.root_id
                       AND snapshot.phase IN (
                         'queued', 'discovering', 'fingerprinting',
                         'classifying', 'parsing', 'ocr', 'publishing'
                       )
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM source_root_deletion AS deletion
                     WHERE deletion.root_id = pdf_reprocess_job.root_id
                       AND deletion.phase NOT IN ('complete', 'failed')
                   )",
                params![
                    task_id.as_str(),
                    now.as_unix_seconds(),
                    root_id.as_str(),
                    parser_contract,
                    MAX_REPROCESS_ATTEMPTS,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(updated)
    }

    pub fn complete_pdf_reprocess_root(
        &self,
        root_id: &SourceRootId,
        task_id: &ImportTaskId,
        parser_contract: &str,
        now: UnixTimestamp,
    ) -> Result<usize>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.connection
            .borrow_mut()
            .execute(
                "UPDATE pdf_reprocess_job
                 SET state = 'complete',
                     updated_at_seconds = MAX(updated_at_seconds, ?1),
                     completed_at_seconds = ?1
                 WHERE root_id = ?2
                   AND parser_contract = ?3
                   AND state = 'scheduled'
                   AND scheduled_task_id = ?4",
                params![
                    now.as_unix_seconds(),
                    root_id.as_str(),
                    parser_contract,
                    task_id.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)
    }

    pub fn requeue_pdf_reprocess_root(
        &self,
        root_id: &SourceRootId,
        task_id: &ImportTaskId,
        parser_contract: &str,
        now: UnixTimestamp,
    ) -> Result<usize>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.connection
            .borrow_mut()
            .execute(
                "UPDATE pdf_reprocess_job
                 SET state = CASE
                         WHEN attempts < ?1 THEN 'queued'
                         ELSE 'cancelled'
                     END,
                     scheduled_task_id = NULL,
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE root_id = ?3
                   AND parser_contract = ?4
                   AND state = 'scheduled'
                   AND scheduled_task_id = ?5",
                params![
                    MAX_REPROCESS_ATTEMPTS,
                    now.as_unix_seconds(),
                    root_id.as_str(),
                    parser_contract,
                    task_id.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)
    }
}
