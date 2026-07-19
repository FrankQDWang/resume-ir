use std::str::FromStr;

use core_domain::{ContentDigest, ResumeVersionId};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::import_processing_contract::disposition_to_storage;
use crate::{
    import_task_status_to_storage, read_import_task, validate_import_scan_scope,
    IdentityInsertOutcome, ImportProcessingContract, ImportProcessingContractId, ImportScanScope,
    ImportSourceDispositionKind, ImportTask, ImportTaskCompletion,
    ImportTaskDispositionBatchOutcome, ImportTaskId, ImportTaskSourceDisposition, ImportTaskStatus,
    MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess,
    MigrationRebuildContractActivation, Result, UnixTimestamp,
    IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT, IMPORT_TASK_COLUMNS,
};

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn insert_import_processing_contract(
        &self,
        contract: &ImportProcessingContract,
    ) -> Result<IdentityInsertOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let connection = self.connection.borrow();
        insert_import_processing_contract_in_connection(&connection, contract)
    }

    pub fn import_processing_contract(
        &self,
        id: &ImportProcessingContractId,
    ) -> Result<Option<ImportProcessingContract>> {
        import_processing_contract_in_connection(&self.connection.borrow(), id)
    }

    pub fn import_task_processing_contract_id(
        &self,
        task_id: &ImportTaskId,
    ) -> Result<Option<ImportProcessingContractId>> {
        self.connection
            .borrow()
            .query_row(
                "SELECT processing_contract_id
                 FROM import_task_contract_binding
                 WHERE import_task_id = ?1",
                params![task_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(|value| ImportProcessingContractId::from_str(&value))
            .transpose()
    }

    /// Returns every running task, including tasks with an observed
    /// cancellation, for normalization by the exclusive processing owner.
    pub fn running_import_tasks_for_owner_normalization(&self) -> Result<Vec<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "SELECT {IMPORT_TASK_COLUMNS} FROM import_task
             WHERE status = ?1 ORDER BY queued_at_seconds, rowid"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![import_task_status_to_storage(
                ImportTaskStatus::Running
            )])
            .map_err(MetaStoreError::storage)?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            tasks.push(read_import_task(row)?);
        }
        Ok(tasks)
    }

    /// Normalizes one exactly observed orphaned running attempt. Uncancelled
    /// work becomes immediately claimable; a cancelled attempt remains
    /// unclaimable. The compare-and-swap includes the observed timestamp so a
    /// newer heartbeat or terminal state is never overwritten.
    pub fn normalize_observed_orphaned_running_import_task(
        &self,
        observed: &ImportTask,
        failed_at: UnixTimestamp,
    ) -> Result<bool>
    where
        Access: MetadataStoreWriteAccess,
    {
        if observed.status != ImportTaskStatus::Running {
            return Err(MetaStoreError::invalid_transition());
        }
        let failed_at_seconds = failed_at
            .as_unix_seconds()
            .max(observed.updated_at.as_unix_seconds());
        let connection = self.connection.borrow();
        let changed = connection
            .execute(
                "UPDATE import_task
                 SET status = CASE
                         WHEN EXISTS (
                             SELECT 1 FROM import_task_cancellation AS cancellation
                             WHERE cancellation.import_task_id = import_task.id
                         ) THEN ?1 ELSE ?2
                     END,
                     started_at_seconds = CASE
                         WHEN EXISTS (
                             SELECT 1 FROM import_task_cancellation AS cancellation
                             WHERE cancellation.import_task_id = import_task.id
                         ) THEN started_at_seconds ELSE NULL
                     END,
                     finished_at_seconds = CASE
                         WHEN EXISTS (
                             SELECT 1 FROM import_task_cancellation AS cancellation
                             WHERE cancellation.import_task_id = import_task.id
                         ) THEN ?3 ELSE NULL
                     END,
                     updated_at_seconds = ?3
                 WHERE id = ?4 AND status = ?5 AND updated_at_seconds = ?6",
                params![
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                    import_task_status_to_storage(ImportTaskStatus::Queued),
                    failed_at_seconds,
                    observed.id.as_str(),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    observed.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(changed == 1)
    }

    /// Activates the only processing contract allowed to close an unpublished
    /// migration rebuild. A blocked runtime invariant may be reopened only by
    /// an exact contract hard cut before any generation exists. Switching
    /// contracts invalidates task-derived state in the same transaction; no
    /// prior immutable ingest identity is relabelled or backfilled.
    pub fn activate_migration_rebuild_contract(
        &self,
        contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<MigrationRebuildContractActivation>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        insert_import_processing_contract_in_connection(&transaction, contract)?;
        let state = transaction
            .query_row(
                "SELECT service_state, generation, repair_reason
                 FROM search_projection_state WHERE state_key = 'default'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(MetaStoreError::storage)?;
        let active = transaction
            .query_row(
                "SELECT active_contract_id FROM migration_rebuild_contract_state
                 WHERE state_key = 'default'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let is_unpublished_rebuild = state.0 == "repairing"
            && state.1.is_none()
            && state.2.as_deref() == Some("migration_rebuild");
        let is_blocked_contract_hard_cut = state.0 == "repair_blocked"
            && state.1.is_none()
            && state.2.as_deref() == Some("runtime_invariant")
            && active
                .as_deref()
                .is_some_and(|active| active != contract.id().as_str());
        if !is_unpublished_rebuild && !is_blocked_contract_hard_cut {
            return Ok(MigrationRebuildContractActivation::Superseded);
        }
        if is_unpublished_rebuild && active.as_deref() == Some(contract.id().as_str()) {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(MigrationRebuildContractActivation::AlreadyActive);
        }

        let running_task_exists = transaction
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM import_task WHERE status = ?1
                 )",
                params![import_task_status_to_storage(ImportTaskStatus::Running)],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        if running_task_exists != 0 {
            return Ok(MigrationRebuildContractActivation::RunningTaskConflict);
        }

        // Completion rows must be removed before their sealed disposition rows;
        // task deletion then cascades the remaining staged state and scopes.
        transaction
            .execute("DELETE FROM import_task_completion", [])
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute("DELETE FROM import_task_source_disposition", [])
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute("DELETE FROM import_task", [])
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "UPDATE migration_rebuild_contract_state
                 SET active_contract_id = ?1, updated_at_seconds = ?2
                 WHERE state_key = 'default'",
                params![contract.id().as_str(), now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        if is_blocked_contract_hard_cut {
            let reopened = transaction
                .execute(
                    "UPDATE search_projection_state
                     SET service_state = 'repairing', repair_reason = 'migration_rebuild',
                         updated_at_seconds = MAX(updated_at_seconds, ?1)
                     WHERE state_key = 'default'
                       AND service_state = 'repair_blocked'
                       AND generation IS NULL
                       AND repair_reason = 'runtime_invariant'",
                    params![now.as_unix_seconds()],
                )
                .map_err(MetaStoreError::storage)?;
            if reopened != 1 {
                return Err(MetaStoreError::storage_invariant());
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(MigrationRebuildContractActivation::Activated)
    }

    /// Stages one bounded, ordinal-ordered source-disposition batch in a single
    /// transaction. Replaying an identical batch is idempotent; any identity
    /// disagreement rolls the entire batch back.
    pub fn stage_import_task_source_dispositions(
        &self,
        task_id: &ImportTaskId,
        contract_id: &ImportProcessingContractId,
        dispositions: &[ImportTaskSourceDisposition],
    ) -> Result<ImportTaskDispositionBatchOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        if dispositions.is_empty() || dispositions.len() > IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT {
            return Err(MetaStoreError::invalid_value(
                "import_task_source_disposition.batch",
            ));
        }
        for (index, disposition) in dispositions.iter().enumerate() {
            validate_disposition_shape(disposition)?;
            if index > 0 && dispositions[index - 1].source_ordinal >= disposition.source_ordinal {
                return Err(MetaStoreError::invalid_value(
                    "import_task_source_disposition.source_ordinal",
                ));
            }
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        if !binding_matches_running(&transaction, task_id, contract_id)? {
            return Err(MetaStoreError::invalid_transition());
        }

        let mut outcome = ImportTaskDispositionBatchOutcome {
            inserted: 0,
            already_present: 0,
        };
        for disposition in dispositions {
            let inserted = transaction
                .execute(
                    "INSERT OR IGNORE INTO import_task_source_disposition (
                    import_task_id, processing_contract_id, source_ordinal,
                    document_id, source_revision_id, resume_version_id, disposition
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        task_id.as_str(),
                        contract_id.as_str(),
                        u64_to_i64(disposition.source_ordinal)?,
                        disposition.document_id.as_str(),
                        disposition.source_revision_id.as_str(),
                        disposition
                            .resume_version_id
                            .as_ref()
                            .map(ResumeVersionId::as_str),
                        disposition_to_storage(disposition.kind),
                    ],
                )
                .map_err(MetaStoreError::storage)?;
            let stored_matches = transaction
                .query_row(
                    "SELECT EXISTS (
                         SELECT 1 FROM import_task_source_disposition
                         WHERE import_task_id = ?1 AND processing_contract_id = ?2
                           AND source_ordinal = ?3 AND document_id = ?4
                           AND source_revision_id = ?5
                           AND resume_version_id IS ?6 AND disposition = ?7
                     )",
                    params![
                        task_id.as_str(),
                        contract_id.as_str(),
                        u64_to_i64(disposition.source_ordinal)?,
                        disposition.document_id.as_str(),
                        disposition.source_revision_id.as_str(),
                        disposition
                            .resume_version_id
                            .as_ref()
                            .map(ResumeVersionId::as_str),
                        disposition_to_storage(disposition.kind),
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(MetaStoreError::storage)?;
            if stored_matches != 1 {
                return Err(MetaStoreError::immutable_identity_conflict(
                    "import_task_source_disposition",
                ));
            }
            if inserted == 1 {
                outcome.inserted += 1;
            } else {
                outcome.already_present += 1;
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    /// Atomically publishes the final scan scope, seals its exact source
    /// disposition manifest, and transitions the bound task to Completed.
    pub fn complete_import_task(
        &self,
        task_id: &ImportTaskId,
        contract_id: &ImportProcessingContractId,
        final_scope: &ImportScanScope,
        completed_at: UnixTimestamp,
    ) -> Result<ImportTaskCompletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        validate_import_scan_scope(final_scope)?;
        if &final_scope.import_task_id != task_id {
            return Err(MetaStoreError::invalid_value(
                "import_scan_scope.import_task_id",
            ));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let task = transaction
            .query_row(
                "SELECT status, root_path, updated_at_seconds
                 FROM import_task WHERE id = ?1",
                params![task_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(|| MetaStoreError::not_found("import_task"))?;
        if task.0 != import_task_status_to_storage(ImportTaskStatus::Running)
            || task.1 != final_scope.canonical_root_path
            || completed_at.as_unix_seconds() < task.2
            || !binding_matches(&transaction, task_id, contract_id)?
        {
            return Err(MetaStoreError::invalid_transition());
        }
        let contract = import_processing_contract_in_connection(&transaction, contract_id)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        validate_exact_source_manifest(
            &transaction,
            task_id,
            contract_id,
            &contract,
            final_scope.files_discovered,
        )?;
        let manifest_digest = source_manifest_digest(
            &transaction,
            task_id,
            contract_id,
            final_scope.files_discovered,
        )?;
        write_final_scope(&transaction, final_scope)?;
        transaction
            .execute(
                "INSERT INTO import_task_completion (
                    import_task_id, processing_contract_id, source_disposition_count,
                    source_manifest_digest, completed_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    task_id.as_str(),
                    contract_id.as_str(),
                    u64_to_i64(final_scope.files_discovered)?,
                    manifest_digest.as_str(),
                    completed_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        let changed = transaction
            .execute(
                "UPDATE import_task
                 SET status = ?1, finished_at_seconds = ?2, updated_at_seconds = ?2
                 WHERE id = ?3 AND status = ?4 AND updated_at_seconds = ?5",
                params![
                    import_task_status_to_storage(ImportTaskStatus::Completed),
                    completed_at.as_unix_seconds(),
                    task_id.as_str(),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    task.2,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        let completion = ImportTaskCompletion {
            import_task_id: task_id.clone(),
            processing_contract_id: contract_id.clone(),
            source_disposition_count: final_scope.files_discovered,
            source_manifest_digest: manifest_digest,
            completed_at,
        };
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(completion)
    }
}

pub(super) fn insert_import_processing_contract_in_connection(
    connection: &Connection,
    contract: &ImportProcessingContract,
) -> Result<IdentityInsertOutcome> {
    let inserted = connection
        .execute(
            "INSERT OR IGNORE INTO import_processing_contract (
                id, primary_parse_version, ocr_parse_version,
                derived_schema_version, classifier_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                contract.id().as_str(),
                contract.primary_parse_version(),
                contract.ocr_parse_version(),
                contract.derived_schema_version(),
                contract.classifier_epoch(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    let stored = import_processing_contract_in_connection(connection, contract.id())?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if stored != *contract {
        return Err(MetaStoreError::immutable_identity_conflict(
            "import_processing_contract",
        ));
    }
    Ok(if inserted == 1 {
        IdentityInsertOutcome::Inserted
    } else {
        IdentityInsertOutcome::AlreadyPresent
    })
}

pub(super) fn import_processing_contract_in_connection(
    connection: &Connection,
    id: &ImportProcessingContractId,
) -> Result<Option<ImportProcessingContract>> {
    connection
        .query_row(
            "SELECT id, primary_parse_version, ocr_parse_version,
                    derived_schema_version, classifier_epoch
             FROM import_processing_contract WHERE id = ?1",
            params![id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(|row| ImportProcessingContract::from_stored_parts(&row.0, row.1, row.2, row.3, row.4))
        .transpose()
}

pub(super) fn binding_matches(
    connection: &Connection,
    task_id: &ImportTaskId,
    contract_id: &ImportProcessingContractId,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM import_task_contract_binding
                WHERE import_task_id = ?1 AND processing_contract_id = ?2
             )",
            params![task_id.as_str(), contract_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value == 1)
        .map_err(MetaStoreError::storage)
}

fn binding_matches_running(
    connection: &Connection,
    task_id: &ImportTaskId,
    contract_id: &ImportProcessingContractId,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM import_task AS task
                JOIN import_task_contract_binding AS binding
                  ON binding.import_task_id = task.id
                WHERE task.id = ?1 AND task.status = ?2
                  AND binding.processing_contract_id = ?3
             )",
            params![
                task_id.as_str(),
                import_task_status_to_storage(ImportTaskStatus::Running),
                contract_id.as_str()
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value == 1)
        .map_err(MetaStoreError::storage)
}

fn validate_disposition_shape(disposition: &ImportTaskSourceDisposition) -> Result<()> {
    let version_required = matches!(
        disposition.kind,
        ImportSourceDispositionKind::Searchable | ImportSourceDispositionKind::Excluded
    );
    if disposition.resume_version_id.is_some() != version_required {
        return Err(MetaStoreError::invalid_value(
            "import_task_source_disposition.resume_version_id",
        ));
    }
    Ok(())
}

fn validate_exact_source_manifest(
    connection: &Connection,
    task_id: &ImportTaskId,
    contract_id: &ImportProcessingContractId,
    contract: &ImportProcessingContract,
    expected_count: u64,
) -> Result<()> {
    let aggregate = connection
        .query_row(
            "SELECT COUNT(*), MIN(disposition.source_ordinal),
                    MAX(disposition.source_ordinal),
                    COALESCE(SUM(CASE
                        WHEN revision.id IS NULL OR document.id IS NULL
                          OR document.is_deleted <> 0 OR document.content_hash IS NULL
                          OR document.content_hash <> revision.content_hash
                        THEN 1
                        WHEN disposition.disposition = 'searchable' AND (
                          version.id IS NULL OR version.schema_version <> ?4
                          OR version.parse_version NOT IN (?5, ?6)
                          OR classification.status IS NULL
                          OR classification.status <> 'resume_candidate'
                        ) THEN 1
                        WHEN disposition.disposition = 'excluded' AND (
                          version.id IS NULL OR version.schema_version <> ?4
                          OR version.parse_version NOT IN (?5, ?6)
                          OR classification.status IS NULL
                          OR classification.status NOT IN ('non_resume', 'needs_review')
                        ) THEN 1
                        WHEN disposition.disposition = 'ocr_backlog' AND (
                          triage.status IS NULL OR triage.status <> 'ocr_backlog'
                        ) THEN 1
                        WHEN disposition.disposition = 'failed' AND (
                          triage.status IS NULL OR triage.status <> 'failed'
                        ) THEN 1
                        ELSE 0
                    END), 0)
             FROM import_task_source_disposition AS disposition
             LEFT JOIN source_revision AS revision
               ON revision.id = disposition.source_revision_id
              AND revision.document_id = disposition.document_id
             LEFT JOIN document
               ON document.id = disposition.document_id
             LEFT JOIN resume_version AS version
               ON version.id = disposition.resume_version_id
              AND version.document_id = disposition.document_id
              AND version.source_revision_id = disposition.source_revision_id
             LEFT JOIN resume_version_classification AS classification
               ON classification.resume_version_id = disposition.resume_version_id
              AND classification.classifier_epoch = ?3
             LEFT JOIN source_revision_triage AS triage
               ON triage.source_revision_id = disposition.source_revision_id
              AND triage.triage_epoch = ?3
             WHERE disposition.import_task_id = ?1
               AND disposition.processing_contract_id = ?2",
            params![
                task_id.as_str(),
                contract_id.as_str(),
                contract.classifier_epoch(),
                contract.derived_schema_version(),
                contract.primary_parse_version(),
                contract.ocr_parse_version(),
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    let count = u64::try_from(aggregate.0).map_err(|_| MetaStoreError::storage_invariant())?;
    let contiguous = if count == 0 {
        aggregate.1.is_none() && aggregate.2.is_none()
    } else {
        aggregate.1 == Some(0)
            && aggregate.2
                == Some(i64::try_from(count - 1).map_err(|_| MetaStoreError::storage_invariant())?)
    };
    if count != expected_count || !contiguous || aggregate.3 != 0 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn source_manifest_digest(
    connection: &Connection,
    task_id: &ImportTaskId,
    contract_id: &ImportProcessingContractId,
    source_disposition_count: u64,
) -> Result<ContentDigest> {
    let mut hasher = Sha256::new();
    update_manifest_part(&mut hasher, b"resume-ir.import-source-manifest.v1")?;
    update_manifest_part(&mut hasher, task_id.as_str().as_bytes())?;
    update_manifest_part(&mut hasher, contract_id.as_str().as_bytes())?;
    hasher.update(source_disposition_count.to_le_bytes());

    let mut statement = connection
        .prepare(
            "SELECT source_ordinal, document_id, source_revision_id,
                    resume_version_id, disposition
             FROM import_task_source_disposition
             WHERE import_task_id = ?1 AND processing_contract_id = ?2
             ORDER BY source_ordinal",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![task_id.as_str(), contract_id.as_str()])
        .map_err(MetaStoreError::storage)?;
    let mut expected_ordinal = 0_u64;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let ordinal = u64::try_from(row.get::<_, i64>(0).map_err(MetaStoreError::storage)?)
            .map_err(|_| MetaStoreError::storage_invariant())?;
        if ordinal != expected_ordinal {
            return Err(MetaStoreError::storage_invariant());
        }
        let document_id = row.get::<_, String>(1).map_err(MetaStoreError::storage)?;
        let source_revision_id = row.get::<_, String>(2).map_err(MetaStoreError::storage)?;
        let resume_version_id = row
            .get::<_, Option<String>>(3)
            .map_err(MetaStoreError::storage)?;
        let disposition = row.get::<_, String>(4).map_err(MetaStoreError::storage)?;
        hasher.update(ordinal.to_le_bytes());
        update_manifest_part(&mut hasher, document_id.as_bytes())?;
        update_manifest_part(&mut hasher, source_revision_id.as_bytes())?;
        if let Some(version_id) = resume_version_id {
            hasher.update([1]);
            update_manifest_part(&mut hasher, version_id.as_bytes())?;
        } else {
            hasher.update([0]);
        }
        update_manifest_part(&mut hasher, disposition.as_bytes())?;
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or_else(MetaStoreError::storage_invariant)?;
    }
    if expected_ordinal != source_disposition_count {
        return Err(MetaStoreError::storage_invariant());
    }
    ContentDigest::from_str(&format!("sha256:{:x}", hasher.finalize()))
        .map_err(|_| MetaStoreError::storage_invariant())
}

#[cfg(test)]
#[path = "import_processing_store_tests.rs"]
mod tests;

fn update_manifest_part(hasher: &mut Sha256, value: &[u8]) -> Result<()> {
    let length = u64::try_from(value.len()).map_err(|_| MetaStoreError::storage_invariant())?;
    hasher.update(length.to_le_bytes());
    hasher.update(value);
    Ok(())
}

fn write_final_scope(transaction: &Transaction<'_>, scope: &ImportScanScope) -> Result<()> {
    let changed = transaction
        .execute(
            "UPDATE import_scan_scope SET
                root_kind = ?1, root_preset = ?2, scan_profile = ?3,
                requested_root_path = ?4, canonical_root_path = ?5,
                files_discovered = ?6, ignored_entries = ?7, scan_errors = ?8,
                searchable_documents = ?9, ocr_required_documents = ?10,
                ocr_jobs_queued = ?11, failed_documents = ?12,
                deleted_documents = ?13, scan_budget_kind = ?14,
                scan_budget_limit = ?15, scan_budget_observed = ?16,
                scan_budget_exhausted = ?17, updated_at_seconds = ?18
             WHERE import_task_id = ?19",
            params![
                crate::import_root_kind_to_storage(scope.root_kind),
                scope.root_preset.map(crate::import_root_preset_to_storage),
                crate::import_scan_profile_to_storage(scope.scan_profile),
                scope.requested_root_path.as_str(),
                scope.canonical_root_path.as_str(),
                u64_to_i64(scope.files_discovered)?,
                u64_to_i64(scope.ignored_entries)?,
                u64_to_i64(scope.scan_errors)?,
                u64_to_i64(scope.searchable_documents)?,
                u64_to_i64(scope.ocr_required_documents)?,
                u64_to_i64(scope.ocr_jobs_queued)?,
                u64_to_i64(scope.failed_documents)?,
                u64_to_i64(scope.deleted_documents)?,
                scope
                    .scan_budget_kind
                    .map(crate::import_scan_budget_kind_to_storage),
                scope.scan_budget_limit.map(u64_to_i64).transpose()?,
                scope.scan_budget_observed.map(u64_to_i64).transpose()?,
                i64::from(scope.scan_budget_exhausted),
                scope.updated_at.as_unix_seconds(),
                scope.import_task_id.as_str(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(MetaStoreError::not_found("import_scan_scope"));
    }
    Ok(())
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}
