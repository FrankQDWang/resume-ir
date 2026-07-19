use rusqlite::{params, TransactionBehavior};

use crate::{
    import_task_status_to_storage, ImportTask, ImportTaskStatus, MetaStoreError, MetadataStore,
    MetadataStoreAccess, MetadataStoreWriteAccess, Result, SearchRepairReason, UnixTimestamp,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTaskFailure {
    Retryable,
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedImportTaskFailureOutcome {
    Applied,
    Superseded,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    /// Fails an exact observed Running task with a compare-and-swap. When that
    /// task belongs to the active unpublished migration rebuild, an optional
    /// terminal repair reason is applied in the same transaction. A same-owner
    /// heartbeat only advances the terminal timestamp; cancellation, pause,
    /// publication, or an owner race supersedes the whole operation without
    /// partially changing either state machine.
    pub fn fail_observed_import_task(
        &self,
        observed: &ImportTask,
        failure: ImportTaskFailure,
        migration_block_reason: Option<SearchRepairReason>,
        failed_at: UnixTimestamp,
    ) -> Result<ObservedImportTaskFailureOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        if observed.status != ImportTaskStatus::Running
            || observed.started_at.is_none()
            || observed.finished_at.is_some()
            || failed_at.as_unix_seconds()
                < observed
                    .started_at
                    .map_or(observed.queued_at.as_unix_seconds(), |value| {
                        value.as_unix_seconds()
                    })
            || migration_block_reason.is_some_and(|reason| {
                !matches!(
                    reason,
                    SearchRepairReason::SourceUnavailable | SearchRepairReason::RuntimeInvariant
                )
            })
        {
            return Err(MetaStoreError::invalid_transition());
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let migration_head = transaction
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM search_projection_state AS projection
                     WHERE projection.state_key = 'default'
                       AND projection.service_state = 'repairing'
                       AND projection.repair_reason = 'migration_rebuild'
                       AND projection.generation IS NULL
                 )",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            == 1;
        if migration_head {
            let active_full_corpus_task = transaction
                .query_row(
                    "SELECT EXISTS (
                         SELECT 1
                         FROM migration_rebuild_full_corpus_task AS purpose
                         JOIN import_task_contract_binding AS binding
                           ON binding.import_task_id = purpose.import_task_id
                          AND binding.processing_contract_id = purpose.processing_contract_id
                         JOIN migration_rebuild_contract_state AS rebuild
                           ON rebuild.state_key = 'default'
                          AND rebuild.active_contract_id = purpose.processing_contract_id
                         WHERE purpose.import_task_id = ?1
                     )",
                    params![observed.id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(MetaStoreError::storage)?
                == 1;
            if !active_full_corpus_task
                || (failure == ImportTaskFailure::Permanent && migration_block_reason.is_none())
            {
                return Err(MetaStoreError::invalid_transition());
            }
        }

        let terminal_status = match failure {
            ImportTaskFailure::Retryable => ImportTaskStatus::FailedRetryable,
            ImportTaskFailure::Permanent => ImportTaskStatus::FailedPermanent,
        };
        let changed = transaction
            .execute(
                "UPDATE import_task
                 SET status = ?1,
                     finished_at_seconds = MAX(updated_at_seconds, ?2),
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE id = ?3 AND root_path = ?4 AND status = ?5
                   AND queued_at_seconds = ?6 AND started_at_seconds = ?7
                   AND finished_at_seconds IS NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM import_task_cancellation AS cancellation
                       WHERE cancellation.import_task_id = import_task.id
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM authorized_import_root AS root
                       WHERE root.canonical_root_path = import_task.root_path
                         AND root.paused = 1
                   )",
                params![
                    import_task_status_to_storage(terminal_status),
                    failed_at.as_unix_seconds(),
                    observed.id.as_str(),
                    observed.root_path,
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    observed.queued_at.as_unix_seconds(),
                    observed.started_at.map(|value| value.as_unix_seconds()),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Ok(ObservedImportTaskFailureOutcome::Superseded);
        }

        if migration_head {
            if let Some(reason) = migration_block_reason {
                let blocked = transaction
                    .execute(
                        "UPDATE search_projection_state
                         SET service_state = 'repair_blocked', repair_reason = ?1,
                             updated_at_seconds = MAX(updated_at_seconds, ?2)
                         WHERE state_key = 'default'
                           AND service_state = 'repairing'
                           AND repair_reason = 'migration_rebuild'
                           AND generation IS NULL",
                        params![
                            repair_reason_to_storage(reason),
                            failed_at.as_unix_seconds()
                        ],
                    )
                    .map_err(MetaStoreError::storage)?;
                if blocked != 1 {
                    return Ok(ObservedImportTaskFailureOutcome::Superseded);
                }
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(ObservedImportTaskFailureOutcome::Applied)
    }
}

fn repair_reason_to_storage(reason: SearchRepairReason) -> &'static str {
    match reason {
        SearchRepairReason::SourceUnavailable => "source_unavailable",
        SearchRepairReason::RuntimeInvariant => "runtime_invariant",
        SearchRepairReason::MigrationRebuild | SearchRepairReason::ArtifactUnavailable => {
            unreachable!("validated before the transaction")
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        EphemeralMetaStore, ImportProcessingContract, ImportRootKind, ImportRootTaskHeadOutcome,
        ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskPurpose,
        ImportTaskStatus, UnixTimestamp, CLASSIFIER_EPOCH,
    };

    use super::{ImportTaskFailure, ObservedImportTaskFailureOutcome};

    #[test]
    fn owner_heartbeat_does_not_supersede_terminal_failure_cas() {
        let (store, contract) = store_and_contract();
        let queued = enqueue_full_corpus_task(&store, &contract, "heartbeat", 10);
        let running = store
            .claim_observed_import_task_for_worker(&queued, UnixTimestamp::from_unix_seconds(11))
            .unwrap()
            .unwrap();
        assert!(store
            .heartbeat_running_import_task(&running.id, UnixTimestamp::from_unix_seconds(20))
            .unwrap());

        assert_eq!(
            store
                .fail_observed_import_task(
                    &running,
                    ImportTaskFailure::Retryable,
                    None,
                    UnixTimestamp::from_unix_seconds(12),
                )
                .unwrap(),
            ObservedImportTaskFailureOutcome::Applied
        );
        let failed = store.import_task_by_id(&running.id).unwrap().unwrap();
        assert_eq!(failed.status, ImportTaskStatus::FailedRetryable);
        assert_eq!(
            failed.finished_at,
            Some(UnixTimestamp::from_unix_seconds(20))
        );
        assert_eq!(failed.updated_at, UnixTimestamp::from_unix_seconds(20));
    }

    #[test]
    fn cancellation_and_pause_supersede_failure_without_partial_block() {
        let (store, contract) = store_and_contract();
        let cancelled = enqueue_full_corpus_task(&store, &contract, "cancelled", 30);
        let cancelled = store
            .claim_observed_import_task_for_worker(&cancelled, UnixTimestamp::from_unix_seconds(31))
            .unwrap()
            .unwrap();
        assert!(store
            .cancel_import_task(&cancelled.id, UnixTimestamp::from_unix_seconds(32))
            .unwrap());
        assert_eq!(
            store
                .fail_observed_import_task(
                    &cancelled,
                    ImportTaskFailure::Permanent,
                    Some(crate::SearchRepairReason::RuntimeInvariant),
                    UnixTimestamp::from_unix_seconds(33),
                )
                .unwrap(),
            ObservedImportTaskFailureOutcome::Superseded
        );
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            crate::SearchProjectionServiceState::Repairing
        );

        let paused = enqueue_full_corpus_task(&store, &contract, "paused", 40);
        let paused = store
            .claim_observed_import_task_for_worker(&paused, UnixTimestamp::from_unix_seconds(41))
            .unwrap()
            .unwrap();
        store
            .pause_import_root(&paused.root_path, UnixTimestamp::from_unix_seconds(42))
            .unwrap();
        assert_eq!(
            store
                .fail_observed_import_task(
                    &paused,
                    ImportTaskFailure::Retryable,
                    None,
                    UnixTimestamp::from_unix_seconds(43),
                )
                .unwrap(),
            ObservedImportTaskFailureOutcome::Superseded
        );
    }

    fn store_and_contract() -> (EphemeralMetaStore, ImportProcessingContract) {
        let store = EphemeralMetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let contract = ImportProcessingContract::new(
            "failure-parser-v1",
            "failure-ocr-v1",
            "failure-schema-v28",
            CLASSIFIER_EPOCH,
        )
        .unwrap();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(1))
            .unwrap();
        (store, contract)
    }

    fn enqueue_full_corpus_task(
        store: &EphemeralMetaStore,
        contract: &ImportProcessingContract,
        label: &str,
        queued_at: i64,
    ) -> ImportTask {
        let root = format!("/synthetic/{label}");
        let queued_at = UnixTimestamp::from_unix_seconds(queued_at);
        let seed = ImportTask {
            id: ImportTaskId::from_non_secret_parts(&[label, "seed"]),
            root_path: root.clone(),
            status: ImportTaskStatus::Queued,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
        };
        store
            .insert_import_task_with_scan_scope(&seed, &scope(&seed), contract)
            .unwrap();
        assert!(store.cancel_import_task(&seed.id, queued_at).unwrap());
        let task_id = ImportTaskId::from_non_secret_parts(&[label, "full-corpus"]);
        assert!(matches!(
            store
                .enqueue_full_corpus_migration_rebuild_root(&root, &task_id, contract, queued_at,)
                .unwrap(),
            ImportRootTaskHeadOutcome::HeadInserted {
                purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
                ..
            }
        ));
        store.import_task_by_id(&task_id).unwrap().unwrap()
    }

    fn scope(task: &ImportTask) -> ImportScanScope {
        ImportScanScope {
            import_task_id: task.id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: task.root_path.clone(),
            canonical_root_path: task.root_path.clone(),
            files_discovered: 0,
            ignored_entries: 0,
            scan_errors: 0,
            searchable_documents: 0,
            ocr_required_documents: 0,
            ocr_jobs_queued: 0,
            failed_documents: 0,
            deleted_documents: 0,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: task.updated_at,
        }
    }
}
