use std::collections::BTreeSet;

use rusqlite::{params, Connection, TransactionBehavior};

use super::{
    import_root_control::{authorized_root_task_scope, import_root_control_status_from_connection},
    import_task_status_to_storage, insert_import_task_with_scan_scope_in_connection,
    read_import_scan_scope, read_import_task, upsert_authorized_import_root_in_connection,
    validate_import_scan_scope, validate_import_task, ImportProcessingContract, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskPurpose, ImportTaskStatus, MetaStoreError, MetadataStore,
    MetadataStoreAccess, MetadataStoreWriteAccess, Result, UnixTimestamp,
    IMPORT_SCAN_SCOPE_COLUMNS, IMPORT_TASK_COLUMNS,
};

pub const IMPORT_ROOT_TASK_HEAD_BATCH_LIMIT: usize = 64;

/// One serialized request to establish the canonical task head for an import root.
#[derive(Clone, Copy)]
pub enum ImportRootTaskHeadRequest<'a> {
    Configured {
        task: &'a ImportTask,
        scope: &'a ImportScanScope,
        processing_contract: &'a ImportProcessingContract,
    },
    MigrationRebuild {
        canonical_root_path: &'a str,
        task_id: &'a ImportTaskId,
        processing_contract: &'a ImportProcessingContract,
        queued_at: UnixTimestamp,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportRootTaskHeadOutcome {
    HeadInserted {
        task: ImportTask,
        scope: ImportScanScope,
        purpose: ImportTaskPurpose,
        cancellation_requests: usize,
    },
    HeadPromoted {
        task: ImportTask,
        scope: ImportScanScope,
        cancellation_requests: usize,
    },
    HeadRetained {
        task: ImportTask,
        scope: ImportScanScope,
        purpose: ImportTaskPurpose,
        cancellation_requests: usize,
    },
    RunningTaskConflict,
    RootPaused,
    MigrationRebuildSuperseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRootTaskHeadBatchRejection {
    RunningTaskConflict,
    RootPaused,
    MigrationRebuildSuperseded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportRootTaskHeadBatchOutcome {
    Committed {
        outcomes: Vec<ImportRootTaskHeadOutcome>,
    },
    Rejected(ImportRootTaskHeadBatchRejection),
}

#[derive(Clone)]
struct CanonicalHeadRecord {
    task: ImportTask,
    cancelled: bool,
    processing_contract_id: Option<String>,
    purpose: ImportTaskPurpose,
    scope: Option<ImportScanScope>,
}

enum PendingTaskRetention<'a> {
    None,
    Canonical(&'a ImportTaskId),
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Establishes one per-root task head under a SQLite `IMMEDIATE` writer lock.
    ///
    /// During an unpublished migration rebuild, configured work is represented
    /// by an exact full-corpus head. A new configured source-change intent
    /// atomically supersedes running or completed migration work instead of
    /// being silently absorbed by an older scan.
    pub fn coordinate_import_root_task_head(
        &self,
        request: ImportRootTaskHeadRequest<'_>,
    ) -> Result<ImportRootTaskHeadOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        match self.coordinate_import_root_task_heads(&[request])? {
            ImportRootTaskHeadBatchOutcome::Committed { mut outcomes } => {
                outcomes.pop().ok_or_else(MetaStoreError::storage_invariant)
            }
            ImportRootTaskHeadBatchOutcome::Rejected(rejection) => Ok(match rejection {
                ImportRootTaskHeadBatchRejection::RunningTaskConflict => {
                    ImportRootTaskHeadOutcome::RunningTaskConflict
                }
                ImportRootTaskHeadBatchRejection::RootPaused => {
                    ImportRootTaskHeadOutcome::RootPaused
                }
                ImportRootTaskHeadBatchRejection::MigrationRebuildSuperseded => {
                    ImportRootTaskHeadOutcome::MigrationRebuildSuperseded
                }
            }),
        }
    }

    /// Coordinates an all-or-nothing set of root heads under one SQLite
    /// `IMMEDIATE` transaction. Any root-level conflict rejects and rolls back
    /// the entire batch, including root authorization and task retirement.
    pub fn coordinate_import_root_task_heads(
        &self,
        requests: &[ImportRootTaskHeadRequest<'_>],
    ) -> Result<ImportRootTaskHeadBatchOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_import_root_task_head_batch(requests)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let mut outcomes = Vec::with_capacity(requests.len());
        for request in requests.iter().copied() {
            let outcome = coordinate_import_root_task_head_in_connection(&transaction, request)?;
            if let Some(rejection) = batch_rejection(&outcome) {
                transaction.rollback().map_err(MetaStoreError::storage)?;
                return Ok(ImportRootTaskHeadBatchOutcome::Rejected(rejection));
            }
            match &outcome {
                ImportRootTaskHeadOutcome::HeadInserted { task, scope, .. }
                | ImportRootTaskHeadOutcome::HeadPromoted { task, scope, .. }
                | ImportRootTaskHeadOutcome::HeadRetained { task, scope, .. }
                    if task.id != scope.import_task_id =>
                {
                    return Err(MetaStoreError::storage_invariant());
                }
                _ => {}
            }
            outcomes.push(outcome);
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(ImportRootTaskHeadBatchOutcome::Committed { outcomes })
    }
}

fn validate_import_root_task_head_batch(requests: &[ImportRootTaskHeadRequest<'_>]) -> Result<()> {
    if requests.is_empty() || requests.len() > IMPORT_ROOT_TASK_HEAD_BATCH_LIMIT {
        return Err(MetaStoreError::invalid_value(
            "import_root_task_head.batch_size",
        ));
    }
    let mut roots = BTreeSet::new();
    for request in requests {
        let root = match request {
            ImportRootTaskHeadRequest::Configured { task, scope, .. } => {
                validate_configured_request(task, scope)?;
                task.root_path.as_str()
            }
            ImportRootTaskHeadRequest::MigrationRebuild {
                canonical_root_path,
                ..
            } => *canonical_root_path,
        };
        if root.is_empty() || !roots.insert(root) {
            return Err(MetaStoreError::invalid_value(
                "import_root_task_head.batch_root",
            ));
        }
    }
    Ok(())
}

fn batch_rejection(
    outcome: &ImportRootTaskHeadOutcome,
) -> Option<ImportRootTaskHeadBatchRejection> {
    match outcome {
        ImportRootTaskHeadOutcome::RunningTaskConflict => {
            Some(ImportRootTaskHeadBatchRejection::RunningTaskConflict)
        }
        ImportRootTaskHeadOutcome::RootPaused => Some(ImportRootTaskHeadBatchRejection::RootPaused),
        ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => {
            Some(ImportRootTaskHeadBatchRejection::MigrationRebuildSuperseded)
        }
        ImportRootTaskHeadOutcome::HeadInserted { .. }
        | ImportRootTaskHeadOutcome::HeadPromoted { .. }
        | ImportRootTaskHeadOutcome::HeadRetained { .. } => None,
    }
}

pub(super) fn coordinate_import_root_task_head_in_connection(
    connection: &Connection,
    request: ImportRootTaskHeadRequest<'_>,
) -> Result<ImportRootTaskHeadOutcome> {
    match request {
        ImportRootTaskHeadRequest::Configured {
            task,
            scope,
            processing_contract,
        } => coordinate_configured_head(connection, task, scope, processing_contract),
        ImportRootTaskHeadRequest::MigrationRebuild {
            canonical_root_path,
            task_id,
            processing_contract,
            queued_at,
        } => coordinate_migration_head(
            connection,
            canonical_root_path,
            task_id,
            processing_contract,
            queued_at,
            MigrationHeadRequestKind::Reconciliation,
        ),
    }
}

pub(super) fn canonical_import_task_head(
    connection: &Connection,
    root_path: &str,
) -> Result<Option<ImportTask>> {
    let sql = format!(
        "SELECT {IMPORT_TASK_COLUMNS}
         FROM import_task
         WHERE root_path = ?1
         ORDER BY rowid DESC
         LIMIT 1"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![root_path])
        .map_err(MetaStoreError::storage)?;
    rows.next()
        .map_err(MetaStoreError::storage)?
        .map(read_import_task)
        .transpose()
}

fn coordinate_configured_head(
    connection: &Connection,
    task: &ImportTask,
    scope: &ImportScanScope,
    contract: &ImportProcessingContract,
) -> Result<ImportRootTaskHeadOutcome> {
    validate_configured_request(task, scope)?;
    let root_status = import_root_control_status_from_connection(connection, &task.root_path)?;
    if root_status == Some(super::ImportRootControlStatus::Paused) {
        return Ok(ImportRootTaskHeadOutcome::RootPaused);
    }

    if let Some(active_contract_id) = unpublished_migration_contract_id(connection)? {
        if active_contract_id != contract.id().as_str() {
            return Ok(ImportRootTaskHeadOutcome::MigrationRebuildSuperseded);
        }
        let pending_head_policy = if root_status == Some(super::ImportRootControlStatus::Active) {
            let previous_configuration =
                authorized_root_task_scope(connection, &task.root_path, &task.id, task.queued_at)?;
            if same_root_configuration(&previous_configuration, scope) {
                ConfiguredPendingHeadPolicy::RetainEquivalent
            } else {
                ConfiguredPendingHeadPolicy::Supersede
            }
        } else {
            ConfiguredPendingHeadPolicy::Supersede
        };
        upsert_authorized_import_root_in_connection(connection, scope)?;
        return coordinate_migration_head(
            connection,
            &task.root_path,
            &task.id,
            contract,
            task.queued_at,
            MigrationHeadRequestKind::ConfiguredSourceChange(pending_head_policy),
        );
    }

    let head = canonical_head_record(connection, &task.root_path)?;
    if let Some(head) = head.as_ref() {
        let running = !head.cancelled && head.task.status == ImportTaskStatus::Running;
        let reusable = !head.cancelled
            && matches!(
                head.task.status,
                ImportTaskStatus::Queued | ImportTaskStatus::FailedRetryable
            )
            && head.purpose == ImportTaskPurpose::ConfiguredCatchUp
            && head.processing_contract_id.as_deref() == Some(contract.id().as_str())
            && head
                .scope
                .as_ref()
                .is_some_and(|stored| same_root_configuration(stored, scope));
        if running {
            return Ok(ImportRootTaskHeadOutcome::RunningTaskConflict);
        }
        if reusable {
            let cancelled = retire_pending_tasks(
                connection,
                &task.root_path,
                PendingTaskRetention::Canonical(&head.task.id),
                task.updated_at,
            )?;
            return Ok(ImportRootTaskHeadOutcome::HeadRetained {
                task: head.task.clone(),
                scope: head
                    .scope
                    .clone()
                    .ok_or_else(MetaStoreError::storage_invariant)?,
                purpose: head.purpose,
                cancellation_requests: cancelled,
            });
        }
    }

    let cancellation_requests = retire_pending_tasks(
        connection,
        &task.root_path,
        PendingTaskRetention::None,
        task.updated_at,
    )?;
    insert_import_task_with_scan_scope_in_connection(connection, task, scope, contract)?;
    Ok(ImportRootTaskHeadOutcome::HeadInserted {
        task: task.clone(),
        scope: scope.clone(),
        purpose: ImportTaskPurpose::ConfiguredCatchUp,
        cancellation_requests,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MigrationHeadRequestKind {
    ConfiguredSourceChange(ConfiguredPendingHeadPolicy),
    Reconciliation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfiguredPendingHeadPolicy {
    RetainEquivalent,
    Supersede,
}

fn coordinate_migration_head(
    connection: &Connection,
    canonical_root_path: &str,
    task_id: &ImportTaskId,
    contract: &ImportProcessingContract,
    queued_at: UnixTimestamp,
    request_kind: MigrationHeadRequestKind,
) -> Result<ImportRootTaskHeadOutcome> {
    if unpublished_migration_contract_id(connection)?.as_deref() != Some(contract.id().as_str()) {
        return Ok(ImportRootTaskHeadOutcome::MigrationRebuildSuperseded);
    }
    match import_root_control_status_from_connection(connection, canonical_root_path)? {
        Some(super::ImportRootControlStatus::Paused) => {
            return Ok(ImportRootTaskHeadOutcome::RootPaused)
        }
        Some(super::ImportRootControlStatus::Active) => {}
        None => return Err(MetaStoreError::not_found("import_root")),
    }

    let authorized_scope =
        authorized_root_task_scope(connection, canonical_root_path, task_id, queued_at)?;
    let mut scope = authorized_scope.clone();
    scope.scan_budget_kind = None;
    scope.scan_budget_limit = None;
    scope.scan_budget_observed = None;
    scope.scan_budget_exhausted = false;
    let head = canonical_head_record(connection, canonical_root_path)?;
    let configured_pending_head_is_equivalent = matches!(
        request_kind,
        MigrationHeadRequestKind::ConfiguredSourceChange(
            ConfiguredPendingHeadPolicy::RetainEquivalent
        )
    );
    if let Some(head) = head.as_ref().filter(|head| {
        !head.cancelled
            && head.processing_contract_id.as_deref() == Some(contract.id().as_str())
            && head.purpose == ImportTaskPurpose::ConfiguredCatchUp
            && matches!(
                head.task.status,
                ImportTaskStatus::Queued | ImportTaskStatus::FailedRetryable
            )
            && head.scope.as_ref().is_some_and(|stored| {
                migration_scope_is_exact(stored) && same_root_configuration(stored, &scope)
            })
            && (request_kind == MigrationHeadRequestKind::Reconciliation
                || configured_pending_head_is_equivalent)
    }) {
        let cancellation_requests = retire_pending_tasks(
            connection,
            canonical_root_path,
            PendingTaskRetention::Canonical(&head.task.id),
            queued_at,
        )?;
        clear_import_task_scan_budget(connection, &head.task.id)?;
        super::import_task_purpose::insert_migration_rebuild_full_corpus_task_marker_in_connection(
            connection,
            &head.task.id,
            contract.id(),
        )?;
        let persisted_scope = import_scan_scope_for_task(connection, &head.task.id)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        return Ok(ImportRootTaskHeadOutcome::HeadPromoted {
            task: head.task.clone(),
            scope: persisted_scope,
            cancellation_requests,
        });
    }
    if let Some(head) = head.as_ref().filter(|head| {
        !head.cancelled
            && head.processing_contract_id.as_deref() == Some(contract.id().as_str())
            && head.purpose == ImportTaskPurpose::MigrationRebuildFullCorpus
            && matches!(
                head.task.status,
                ImportTaskStatus::Queued | ImportTaskStatus::FailedRetryable
            )
            && head.scope.as_ref().is_some_and(|stored| {
                migration_scope_is_exact(stored) && same_root_configuration(stored, &scope)
            })
            && (request_kind == MigrationHeadRequestKind::Reconciliation
                || configured_pending_head_is_equivalent)
    }) {
        let cancellation_requests = retire_pending_tasks(
            connection,
            canonical_root_path,
            PendingTaskRetention::Canonical(&head.task.id),
            queued_at,
        )?;
        return Ok(ImportRootTaskHeadOutcome::HeadRetained {
            task: head.task.clone(),
            scope: head
                .scope
                .clone()
                .ok_or_else(MetaStoreError::storage_invariant)?,
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            cancellation_requests,
        });
    }
    if let Some(head) = head.as_ref().filter(|head| {
        !head.cancelled
            && head.processing_contract_id.as_deref() == Some(contract.id().as_str())
            && head.purpose == ImportTaskPurpose::MigrationRebuildFullCorpus
            && matches!(
                head.task.status,
                ImportTaskStatus::Running | ImportTaskStatus::Completed
            )
            && head.scope.as_ref().is_some_and(|stored| {
                migration_scope_is_exact(stored) && same_root_configuration(stored, &scope)
            })
            && request_kind == MigrationHeadRequestKind::Reconciliation
    }) {
        let cancellation_requests = retire_pending_tasks(
            connection,
            canonical_root_path,
            PendingTaskRetention::Canonical(&head.task.id),
            queued_at,
        )?;
        return Ok(ImportRootTaskHeadOutcome::HeadRetained {
            task: head.task.clone(),
            scope: head
                .scope
                .clone()
                .ok_or_else(MetaStoreError::storage_invariant)?,
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            cancellation_requests,
        });
    }
    if request_kind == MigrationHeadRequestKind::Reconciliation
        && head
            .as_ref()
            .is_some_and(|head| !head.cancelled && head.task.status == ImportTaskStatus::Running)
    {
        return Ok(ImportRootTaskHeadOutcome::RunningTaskConflict);
    }

    let completed_cancellation_requests = if matches!(
        request_kind,
        MigrationHeadRequestKind::ConfiguredSourceChange(_)
    ) {
        cancel_superseded_completed_head(connection, head.as_ref(), queued_at)?
    } else {
        0
    };
    let cancellation_requests = completed_cancellation_requests
        + retire_pending_tasks(
            connection,
            canonical_root_path,
            PendingTaskRetention::None,
            queued_at,
        )?;
    let task = ImportTask {
        id: task_id.clone(),
        root_path: canonical_root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at,
        started_at: None,
        finished_at: None,
        updated_at: queued_at,
    };
    insert_import_task_with_scan_scope_in_connection(
        connection,
        &task,
        &authorized_scope,
        contract,
    )?;
    clear_import_task_scan_budget(connection, task_id)?;
    super::import_task_purpose::insert_migration_rebuild_full_corpus_task_marker_in_connection(
        connection,
        task_id,
        contract.id(),
    )?;
    Ok(ImportRootTaskHeadOutcome::HeadInserted {
        task,
        scope,
        purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
        cancellation_requests,
    })
}

fn validate_configured_request(task: &ImportTask, scope: &ImportScanScope) -> Result<()> {
    validate_import_task(task)?;
    validate_import_scan_scope(scope)?;
    if task.status != ImportTaskStatus::Queued
        || task.id != scope.import_task_id
        || task.root_path != scope.canonical_root_path
    {
        return Err(MetaStoreError::invalid_value(
            "import_root_task_head.request",
        ));
    }
    Ok(())
}

fn unpublished_migration_contract_id(connection: &Connection) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT CASE
                 WHEN projection.service_state = 'repairing'
                  AND projection.generation IS NULL
                  AND projection.repair_reason = 'migration_rebuild'
                 THEN rebuild.active_contract_id
                 ELSE NULL
             END
             FROM search_projection_state AS projection
             JOIN migration_rebuild_contract_state AS rebuild
               ON rebuild.state_key = projection.state_key
             WHERE projection.state_key = 'default'",
            [],
            |row| row.get(0),
        )
        .map_err(MetaStoreError::storage)
}

fn canonical_head_record(
    connection: &Connection,
    canonical_root_path: &str,
) -> Result<Option<CanonicalHeadRecord>> {
    let sql = format!(
        "SELECT {IMPORT_TASK_COLUMNS},
                EXISTS (
                    SELECT 1 FROM import_task_cancellation AS cancellation
                    WHERE cancellation.import_task_id = import_task.id
                ),
                binding.processing_contract_id,
                CASE WHEN purpose.import_task_id IS NULL THEN 0 ELSE 1 END
         FROM import_task
         LEFT JOIN import_task_contract_binding AS binding
           ON binding.import_task_id = import_task.id
         LEFT JOIN migration_rebuild_full_corpus_task AS purpose
           ON purpose.import_task_id = import_task.id
         WHERE import_task.root_path = ?1
         ORDER BY import_task.rowid DESC
         LIMIT 1"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![canonical_root_path])
        .map_err(MetaStoreError::storage)?;
    let Some(row) = rows.next().map_err(MetaStoreError::storage)? else {
        return Ok(None);
    };
    let task = read_import_task(row)?;
    let cancelled = row.get::<_, i64>(7).map_err(MetaStoreError::storage)?;
    let processing_contract_id = row
        .get::<_, Option<String>>(8)
        .map_err(MetaStoreError::storage)?;
    let migration_purpose = row.get::<_, i64>(9).map_err(MetaStoreError::storage)?;
    let scope = import_scan_scope_for_task(connection, &task.id)?;
    Ok(Some(CanonicalHeadRecord {
        task,
        cancelled: cancelled == 1,
        processing_contract_id,
        purpose: if migration_purpose == 1 {
            ImportTaskPurpose::MigrationRebuildFullCorpus
        } else {
            ImportTaskPurpose::ConfiguredCatchUp
        },
        scope,
    }))
}

fn import_scan_scope_for_task(
    connection: &Connection,
    task_id: &ImportTaskId,
) -> Result<Option<ImportScanScope>> {
    let sql = format!(
        "SELECT {IMPORT_SCAN_SCOPE_COLUMNS}
         FROM import_scan_scope WHERE import_task_id = ?1"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![task_id.as_str()])
        .map_err(MetaStoreError::storage)?;
    rows.next()
        .map_err(MetaStoreError::storage)?
        .map(read_import_scan_scope)
        .transpose()
}

fn same_root_configuration(left: &ImportScanScope, right: &ImportScanScope) -> bool {
    left.root_kind == right.root_kind
        && left.root_preset == right.root_preset
        && left.scan_profile == right.scan_profile
        && left.canonical_root_path == right.canonical_root_path
        && left.scan_budget_kind == right.scan_budget_kind
        && left.scan_budget_limit == right.scan_budget_limit
}

fn migration_scope_is_exact(scope: &ImportScanScope) -> bool {
    scope.scan_budget_kind.is_none()
        && scope.scan_budget_limit.is_none()
        && scope.scan_budget_observed.is_none()
        && !scope.scan_budget_exhausted
}

fn clear_import_task_scan_budget(connection: &Connection, task_id: &ImportTaskId) -> Result<()> {
    let changed = connection
        .execute(
            "UPDATE import_scan_scope
             SET scan_budget_kind = NULL, scan_budget_limit = NULL,
                 scan_budget_observed = NULL, scan_budget_exhausted = 0
             WHERE import_task_id = ?1",
            params![task_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn cancel_superseded_completed_head(
    connection: &Connection,
    head: Option<&CanonicalHeadRecord>,
    requested_at: UnixTimestamp,
) -> Result<usize> {
    let Some(head) =
        head.filter(|head| !head.cancelled && head.task.status == ImportTaskStatus::Completed)
    else {
        return Ok(0);
    };
    connection
        .execute(
            "INSERT OR IGNORE INTO import_task_cancellation (
                 import_task_id, requested_at_seconds
             ) VALUES (?1, ?2)",
            params![
                head.task.id.as_str(),
                requested_at
                    .as_unix_seconds()
                    .max(head.task.updated_at.as_unix_seconds()),
            ],
        )
        .map_err(MetaStoreError::storage)
}

fn retire_pending_tasks(
    connection: &Connection,
    canonical_root_path: &str,
    retention: PendingTaskRetention<'_>,
    requested_at: UnixTimestamp,
) -> Result<usize> {
    let retained_id = match retention {
        PendingTaskRetention::None => "",
        PendingTaskRetention::Canonical(task_id) => task_id.as_str(),
    };
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO import_task_cancellation (
                 import_task_id, requested_at_seconds
             )
             SELECT id, MAX(updated_at_seconds, ?1)
             FROM import_task
             WHERE root_path = ?2 AND id <> ?3
               AND status IN (?4, ?5, ?6)
               AND NOT EXISTS (
                   SELECT 1 FROM import_task_cancellation AS cancellation
                   WHERE cancellation.import_task_id = import_task.id
               )",
            params![
                requested_at.as_unix_seconds(),
                canonical_root_path,
                retained_id,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    connection
        .execute(
            "UPDATE import_task
             SET updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE root_path = ?2 AND id <> ?3
               AND status IN (?4, ?5, ?6)
               AND EXISTS (
                   SELECT 1 FROM import_task_cancellation AS cancellation
                   WHERE cancellation.import_task_id = import_task.id
               )",
            params![
                requested_at.as_unix_seconds(),
                canonical_root_path,
                retained_id,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(changed)
}

#[cfg(test)]
#[path = "import_root_head_tests.rs"]
mod tests;
