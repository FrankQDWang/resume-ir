use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{
    import_root_head::{
        coordinate_import_root_task_head_in_connection, ImportRootTaskHeadOutcome,
        ImportRootTaskHeadRequest,
    },
    import_root_kind_from_storage, import_root_preset_from_storage,
    import_scan_budget_kind_from_storage, import_scan_profile_from_storage,
    import_task_status_to_storage, ImportProcessingContract, ImportScanScope, ImportTask,
    ImportTaskId, ImportTaskStatus, MetaStoreError, MetadataStore, MetadataStoreAccess,
    MetadataStoreWriteAccess, Result, UnixTimestamp,
};

pub(super) const SCHEMA_V26: &str = r#"
CREATE TABLE import_root_control (
    canonical_root_path TEXT PRIMARY KEY NOT NULL CHECK (length(canonical_root_path) > 0),
    paused INTEGER NOT NULL CHECK (paused IN (0, 1)),
    updated_at_seconds INTEGER NOT NULL
);

CREATE INDEX import_root_control_paused_idx
    ON import_root_control(paused, updated_at_seconds);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRootControlStatus {
    Active,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportRootControlUpdate {
    pub status: ImportRootControlStatus,
    pub changed: bool,
    pub cancellation_requests: usize,
    pub catch_up_queued: bool,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn active_authorized_import_roots(&self) -> Result<Vec<String>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT canonical_root_path
                 FROM authorized_import_root
                 WHERE paused = 0
                 ORDER BY canonical_root_path",
            )
            .map_err(MetaStoreError::storage)?;
        let roots = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        Ok(roots)
    }

    pub fn enqueue_authorized_import_root(
        &self,
        canonical_root_path: &str,
        task_id: &ImportTaskId,
        contract: &ImportProcessingContract,
        updated_at: UnixTimestamp,
    ) -> Result<ImportRootTaskHeadOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let scope =
            authorized_root_task_scope(&transaction, canonical_root_path, task_id, updated_at)?;
        let task = queued_task(task_id, canonical_root_path, updated_at);
        let outcome = coordinate_import_root_task_head_in_connection(
            &transaction,
            ImportRootTaskHeadRequest::Configured {
                task: &task,
                scope: &scope,
                processing_contract: contract,
            },
        )?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    /// Enqueues an authorized root for a complete migration rebuild scan.
    ///
    /// The task scope is deliberately unbounded even when the root's normal
    /// catch-up configuration has a scan budget. The stored root configuration
    /// remains unchanged for later non-migration imports.
    pub fn enqueue_full_corpus_migration_rebuild_root(
        &self,
        canonical_root_path: &str,
        task_id: &ImportTaskId,
        contract: &ImportProcessingContract,
        updated_at: UnixTimestamp,
    ) -> Result<ImportRootTaskHeadOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.coordinate_import_root_task_head(ImportRootTaskHeadRequest::MigrationRebuild {
            canonical_root_path,
            task_id,
            processing_contract: contract,
            queued_at: updated_at,
        })
    }

    pub fn import_root_control_status(
        &self,
        canonical_root_path: &str,
    ) -> Result<Option<ImportRootControlStatus>> {
        let connection = self.connection.borrow();
        import_root_control_status_from_connection(&connection, canonical_root_path)
    }

    pub fn pause_import_root(
        &self,
        canonical_root_path: &str,
        updated_at: UnixTimestamp,
    ) -> Result<ImportRootControlUpdate>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let status = require_known_root(&transaction, canonical_root_path)?;
        let changed = status == ImportRootControlStatus::Active;
        let updated_at_seconds = updated_at.as_unix_seconds();

        transaction
            .execute(
                "\
                UPDATE authorized_import_root
                SET paused = 1, updated_at_seconds = ?2
                WHERE canonical_root_path = ?1",
                params![canonical_root_path, updated_at_seconds],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                UPDATE import_task
                SET updated_at_seconds = MAX(updated_at_seconds, ?1)
                WHERE root_path = ?2
                    AND status IN (?3, ?4, ?5)",
                params![
                    updated_at_seconds,
                    canonical_root_path,
                    import_task_status_to_storage(ImportTaskStatus::Queued),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        let cancellation_requests = transaction
            .execute(
                "\
                INSERT OR IGNORE INTO import_task_cancellation (
                    import_task_id, requested_at_seconds
                )
                SELECT id, ?1
                FROM import_task
                WHERE root_path = ?2
                    AND status IN (?3, ?4, ?5)",
                params![
                    updated_at_seconds,
                    canonical_root_path,
                    import_task_status_to_storage(ImportTaskStatus::Queued),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        Ok(ImportRootControlUpdate {
            status: ImportRootControlStatus::Paused,
            changed,
            cancellation_requests,
            catch_up_queued: false,
        })
    }

    pub fn resume_import_root(
        &self,
        canonical_root_path: &str,
        catch_up_task_id: &ImportTaskId,
        contract: &ImportProcessingContract,
        updated_at: UnixTimestamp,
    ) -> Result<ImportRootControlUpdate>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let status = require_known_root(&transaction, canonical_root_path)?;
        if status == ImportRootControlStatus::Active {
            return Ok(ImportRootControlUpdate {
                status,
                changed: false,
                cancellation_requests: 0,
                catch_up_queued: false,
            });
        }

        transaction
            .execute(
                "\
                UPDATE authorized_import_root
                SET paused = 0, updated_at_seconds = ?1
                WHERE canonical_root_path = ?2",
                params![updated_at.as_unix_seconds(), canonical_root_path],
            )
            .map_err(MetaStoreError::storage)?;
        let scope = authorized_root_task_scope(
            &transaction,
            canonical_root_path,
            catch_up_task_id,
            updated_at,
        )?;
        let task = queued_task(catch_up_task_id, canonical_root_path, updated_at);
        let outcome = coordinate_import_root_task_head_in_connection(
            &transaction,
            ImportRootTaskHeadRequest::Configured {
                task: &task,
                scope: &scope,
                processing_contract: contract,
            },
        )?;
        let catch_up_queued = matches!(outcome, ImportRootTaskHeadOutcome::HeadInserted { .. });
        transaction.commit().map_err(MetaStoreError::storage)?;

        Ok(ImportRootControlUpdate {
            status: ImportRootControlStatus::Active,
            changed: true,
            cancellation_requests: 0,
            catch_up_queued,
        })
    }
}

fn require_known_root(
    connection: &Connection,
    canonical_root_path: &str,
) -> Result<ImportRootControlStatus> {
    import_root_control_status_from_connection(connection, canonical_root_path)?
        .ok_or_else(|| MetaStoreError::not_found("import_root"))
}

pub(super) fn import_root_control_status_from_connection(
    connection: &Connection,
    canonical_root_path: &str,
) -> Result<Option<ImportRootControlStatus>> {
    let paused = connection
        .query_row(
            "SELECT paused FROM authorized_import_root WHERE canonical_root_path = ?1",
            params![canonical_root_path],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    match paused {
        None => Ok(None),
        Some(0) => Ok(Some(ImportRootControlStatus::Active)),
        Some(1) => Ok(Some(ImportRootControlStatus::Paused)),
        _ => Err(MetaStoreError::invalid_value("import_root_control.paused")),
    }
}

pub(super) fn authorized_root_task_scope(
    connection: &Connection,
    canonical_root_path: &str,
    task_id: &ImportTaskId,
    updated_at: UnixTimestamp,
) -> Result<ImportScanScope> {
    connection
        .query_row(
            "SELECT root_kind, root_preset, scan_profile, requested_root_path,
                    canonical_root_path, scan_budget_kind, scan_budget_limit
             FROM authorized_import_root
             WHERE canonical_root_path = ?1",
            params![canonical_root_path],
            |row| {
                let root_kind = import_root_kind_from_storage(&row.get::<_, String>(0)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let root_preset = row
                    .get::<_, Option<String>>(1)?
                    .as_deref()
                    .map(import_root_preset_from_storage)
                    .transpose()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let scan_profile = import_scan_profile_from_storage(&row.get::<_, String>(2)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let scan_budget_kind = row
                    .get::<_, Option<String>>(5)?
                    .as_deref()
                    .map(import_scan_budget_kind_from_storage)
                    .transpose()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let scan_budget_limit = row
                    .get::<_, Option<i64>>(6)?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(ImportScanScope {
                    import_task_id: task_id.clone(),
                    root_kind,
                    root_preset,
                    scan_profile,
                    requested_root_path: row.get(3)?,
                    canonical_root_path: row.get(4)?,
                    files_discovered: 0,
                    ignored_entries: 0,
                    scan_errors: 0,
                    searchable_documents: 0,
                    ocr_required_documents: 0,
                    ocr_jobs_queued: 0,
                    failed_documents: 0,
                    deleted_documents: 0,
                    scan_budget_kind,
                    scan_budget_limit,
                    scan_budget_observed: scan_budget_limit.map(|_| 0),
                    scan_budget_exhausted: false,
                    updated_at,
                })
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::not_found("import_root"))
}

fn queued_task(
    task_id: &ImportTaskId,
    canonical_root_path: &str,
    updated_at: UnixTimestamp,
) -> ImportTask {
    ImportTask {
        id: task_id.clone(),
        root_path: canonical_root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: updated_at,
        started_at: None,
        finished_at: None,
        updated_at,
    }
}
