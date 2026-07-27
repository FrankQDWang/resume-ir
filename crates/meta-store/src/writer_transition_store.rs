//! Durable writer-contract transition and reprocessing-campaign store API.

use std::str::FromStr;

use core_domain::ContentDigest;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::contract_delta::{ContractDelta, ContractTransitionStrategy};
use crate::import_processing_store::insert_import_processing_contract_in_connection;
use crate::import_root_control::authorized_root_task_scope;
use crate::processing_contract_transition::{
    campaign_domain_for, observe_writer_contract_transition, WriterAuthorityHealthState,
    WriterAuthoritySnapshot, WriterContractTransitionOutcome, WriterTransitionPhase,
    WriterTransitionReceipt,
};
use crate::{
    import_task_status_to_storage, ImportProcessingContract, ImportProcessingContractId,
    ImportTask, ImportTaskId, ImportTaskStatus, MetaStoreError, MetadataStore, MetadataStoreAccess,
    MetadataStoreWriteAccess, Result, UnixTimestamp,
};

const PRODUCT_VERSION_DEFAULT: &str = "0.1.9";

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn writer_authority_snapshot(&self) -> Result<WriterAuthoritySnapshot> {
        read_authority_snapshot(&self.connection.borrow())
    }

    /// Whether ordinary public writers may claim or enqueue work.
    pub fn public_writer_claims_admitted(&self) -> Result<bool> {
        let snapshot = self.writer_authority_snapshot()?;
        Ok(snapshot.health_state == WriterAuthorityHealthState::Ready)
    }

    /// Records a transient runtime probe failure without clearing an in-flight
    /// transition. Cleared by [`Self::reconcile_writer_runtime_availability`]
    /// once the probe succeeds again.
    pub fn mark_writer_runtime_unavailable(&self, now: UnixTimestamp) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let connection = self.connection.borrow();
        connection
            .execute(
                "UPDATE writer_authority_state
                 SET health_state = 'unavailable',
                     health_reason = 'runtime_unavailable',
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'",
                params![now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(())
    }

    /// Clears a prior runtime_unavailable latch when the probe is healthy again.
    pub fn reconcile_writer_runtime_availability(
        &self,
        runtime_healthy: bool,
        now: UnixTimestamp,
    ) -> Result<bool>
    where
        Access: MetadataStoreWriteAccess,
    {
        if !runtime_healthy {
            return Ok(false);
        }
        let connection = self.connection.borrow();
        let changed = connection
            .execute(
                "UPDATE writer_authority_state
                 SET health_state = CASE
                         WHEN active_transition_id IS NOT NULL THEN 'transitioning'
                         ELSE 'ready'
                     END,
                     health_reason = CASE
                         WHEN active_transition_id IS NOT NULL THEN 'transition_in_progress'
                         ELSE NULL
                     END,
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'
                   AND health_state = 'unavailable'
                   AND health_reason = 'runtime_unavailable'",
                params![now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(changed == 1)
    }

    /// Writer-only barrier for a contended orphaned running task owner.
    pub fn mark_writer_blocked_by_running_owner(&self, now: UnixTimestamp) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let connection = self.connection.borrow();
        connection
            .execute(
                "UPDATE writer_authority_state
                 SET health_state = 'blocked',
                     health_reason = 'blocked_by_running_owner',
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'",
                params![now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(())
    }

    /// Clears a prior blocked_by_running_owner latch after orphan ownership is free.
    pub fn clear_writer_blocked_by_running_owner(&self, now: UnixTimestamp) -> Result<bool>
    where
        Access: MetadataStoreWriteAccess,
    {
        let connection = self.connection.borrow();
        let changed = connection
            .execute(
                "UPDATE writer_authority_state
                 SET health_state = CASE
                         WHEN active_transition_id IS NOT NULL THEN 'transitioning'
                         ELSE 'ready'
                     END,
                     health_reason = CASE
                         WHEN active_transition_id IS NOT NULL THEN 'transition_in_progress'
                         ELSE NULL
                     END,
                     updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE state_key = 'default'
                   AND health_state = 'blocked'
                   AND health_reason = 'blocked_by_running_owner'",
                params![now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(changed == 1)
    }

    /// Runs the online Ready-path transition to WriterReady, or reports why not.
    ///
    /// Replaces the former shortcut that only updated
    /// `migration_rebuild_contract_state` without a fence or campaign.
    pub fn complete_online_writer_transition(
        &self,
        desired: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<WriterContractTransitionOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.complete_online_writer_transition_with_product(
            desired,
            PRODUCT_VERSION_DEFAULT,
            /*desired_schema_version*/ 34,
            now,
        )
    }

    pub fn complete_online_writer_transition_with_product(
        &self,
        desired: &ImportProcessingContract,
        product_version: &str,
        desired_schema_version: u32,
        now: UnixTimestamp,
    ) -> Result<WriterContractTransitionOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let outcome = complete_online_writer_transition_in_transaction(
            &transaction,
            desired,
            product_version,
            desired_schema_version,
            now,
        )?;
        match outcome {
            WriterContractTransitionOutcome::BlockedByRunningOwner
            | WriterContractTransitionOutcome::PersistedStateInvalid
            | WriterContractTransitionOutcome::UnsupportedTransition
            | WriterContractTransitionOutcome::RuntimeUnavailable
            | WriterContractTransitionOutcome::TransitionRequired
            | WriterContractTransitionOutcome::TransitionInProgress => {
                // Attempt failures that leave durable retryable state still commit;
                // terminal invalid / unsupported roll back unless already persisted.
                if matches!(
                    outcome,
                    WriterContractTransitionOutcome::BlockedByRunningOwner
                        | WriterContractTransitionOutcome::TransitionInProgress
                ) {
                    transaction.commit().map_err(MetaStoreError::storage)?;
                }
                Ok(outcome)
            }
            WriterContractTransitionOutcome::AlreadyActive
            | WriterContractTransitionOutcome::TargetCommitted => {
                transaction.commit().map_err(MetaStoreError::storage)?;
                Ok(outcome)
            }
        }
    }

    pub fn recover_writer_contract_transition_on_open(
        &self,
        desired: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<WriterContractTransitionOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        self.complete_online_writer_transition(desired, now)
    }
}

fn complete_online_writer_transition_in_transaction(
    transaction: &Transaction<'_>,
    desired: &ImportProcessingContract,
    product_version: &str,
    desired_schema_version: u32,
    now: UnixTimestamp,
) -> Result<WriterContractTransitionOutcome> {
    insert_import_processing_contract_in_connection(transaction, desired)?;
    if !projection_is_ready_with_generation(transaction)? {
        return Ok(WriterContractTransitionOutcome::PersistedStateInvalid);
    }

    // Active transition always wins over the AlreadyActive fast path so a
    // TargetCommitted-but-unmaterialized crash resumes the campaign.
    if let Some(existing) = active_transition_row(transaction)? {
        if existing.target_contract_id.as_str() != desired.id().as_str() {
            return Ok(WriterContractTransitionOutcome::TransitionInProgress);
        }
        // After TargetCommitted, legacy active already equals desired. Recompute
        // strategy from the transition source so campaign materialization still runs.
        let source = match existing.source_contract_id.as_deref() {
            Some(id) => {
                let contract_id = ImportProcessingContractId::from_str(id)?;
                crate::import_processing_store::import_processing_contract_in_connection(
                    transaction,
                    &contract_id,
                )?
            }
            None => None,
        };
        let running = running_count(transaction)?;
        let (delta, _) = observe_writer_contract_transition(source.as_ref(), desired, running);
        if delta.strategy == ContractTransitionStrategy::Unsupported
            && existing.phase != WriterTransitionPhase::TargetCommitted
            && existing.phase != WriterTransitionPhase::WriterReady
        {
            return Ok(WriterContractTransitionOutcome::UnsupportedTransition);
        }
        return advance_existing_transition(transaction, &existing, &delta, desired, now);
    }

    let committed = active_contract(transaction)?;
    let running = running_count(transaction)?;
    let (delta, observed) =
        observe_writer_contract_transition(committed.as_ref(), desired, running);
    if observed == WriterContractTransitionOutcome::AlreadyActive {
        sync_authority_ready(transaction, desired.id(), now)?;
        return Ok(WriterContractTransitionOutcome::AlreadyActive);
    }
    if observed == WriterContractTransitionOutcome::UnsupportedTransition
        || delta.strategy == ContractTransitionStrategy::Unsupported
    {
        return Ok(WriterContractTransitionOutcome::UnsupportedTransition);
    }

    if running > 0 {
        record_blocked_attempt(
            transaction,
            desired,
            product_version,
            desired_schema_version,
            now,
        )?;
        return Ok(WriterContractTransitionOutcome::BlockedByRunningOwner);
    }

    let transition_id = new_digest_id(&[
        b"writer-contract-transition",
        desired.id().as_str().as_bytes(),
        &now.as_unix_seconds().to_le_bytes(),
    ]);
    let source_id = committed.as_ref().map(|c| c.id().as_str().to_string());
    let (generation, visible_epoch) = projection_identity(transaction)?;
    let claim_fence_epoch = next_claim_fence_epoch(transaction)?;
    let queued = queued_count(transaction)?;

    transaction
        .execute(
            "INSERT INTO writer_contract_transition (
                transition_id, source_contract_id, target_contract_id,
                desired_product_version, desired_schema_version,
                source_generation, source_visible_epoch, phase, attempt,
                claim_fence_epoch, running_task_count, queued_task_count,
                scheduled_task_count, created_at_seconds, updated_at_seconds
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'observed', 1, ?8, ?9, ?10, 0, ?11, ?11
             )",
            params![
                transition_id.as_str(),
                source_id.as_deref(),
                desired.id().as_str(),
                product_version,
                i64::from(desired_schema_version),
                generation.as_deref(),
                visible_epoch,
                i64::try_from(claim_fence_epoch).map_err(|_| MetaStoreError::storage_invariant())?,
                i64::try_from(running).map_err(|_| MetaStoreError::storage_invariant())?,
                i64::try_from(queued).map_err(|_| MetaStoreError::storage_invariant())?,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    set_authority_transitioning(
        transaction,
        &transition_id,
        WriterTransitionPhase::Observed,
        claim_fence_epoch,
        source_id.as_deref(),
        desired.id().as_str(),
        now,
    )?;

    let row = TransitionRow {
        transition_id: transition_id.clone(),
        source_contract_id: source_id.clone(),
        target_contract_id: desired.id().as_str().to_string(),
        phase: WriterTransitionPhase::Observed,
        claim_fence_epoch,
        attempt: 1,
    };
    advance_existing_transition(transaction, &row, &delta, desired, now)
}

fn advance_existing_transition(
    transaction: &Transaction<'_>,
    row: &TransitionRow,
    delta: &ContractDelta,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<WriterContractTransitionOutcome> {
    let mut phase = row.phase;
    if phase == WriterTransitionPhase::Observed {
        fence_claims(transaction, row, now)?;
        phase = WriterTransitionPhase::ClaimsFenced;
    }
    if phase == WriterTransitionPhase::ClaimsFenced {
        let running = running_count(transaction)?;
        if running > 0 {
            mark_transition_failure(
                transaction,
                &row.transition_id,
                "blocked_by_running_owner",
                /*retryable*/ true,
                now,
            )?;
            transaction
                .execute(
                    "UPDATE writer_authority_state
                     SET health_reason = 'blocked_by_running_owner',
                         updated_at_seconds = MAX(updated_at_seconds, ?1)
                     WHERE state_key = 'default'",
                    params![now.as_unix_seconds()],
                )
                .map_err(MetaStoreError::storage)?;
            return Ok(WriterContractTransitionOutcome::BlockedByRunningOwner);
        }
        set_phase(
            transaction,
            &row.transition_id,
            WriterTransitionPhase::WorkersQuiesced,
            now,
        )?;
        set_authority_phase(transaction, WriterTransitionPhase::WorkersQuiesced, now)?;
        phase = WriterTransitionPhase::WorkersQuiesced;
    }
    if phase == WriterTransitionPhase::WorkersQuiesced {
        commit_target_contract(transaction, row, desired, now)?;
        phase = WriterTransitionPhase::TargetCommitted;
    }
    if phase == WriterTransitionPhase::TargetCommitted {
        materialize_campaign(transaction, row, delta.strategy, desired, now)?;
        set_phase(
            transaction,
            &row.transition_id,
            WriterTransitionPhase::WriterReady,
            now,
        )?;
        transaction
            .execute(
                "UPDATE writer_contract_transition
                 SET completed_at_seconds = ?1, updated_at_seconds = ?1
                 WHERE transition_id = ?2",
                params![now.as_unix_seconds(), row.transition_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        set_authority_ready_after_transition(transaction, desired.id(), &row.transition_id, now)?;
        return Ok(WriterContractTransitionOutcome::TargetCommitted);
    }
    if phase == WriterTransitionPhase::WriterReady {
        sync_authority_ready(transaction, desired.id(), now)?;
        return Ok(WriterContractTransitionOutcome::AlreadyActive);
    }
    Ok(WriterContractTransitionOutcome::TransitionInProgress)
}

fn fence_claims(
    transaction: &Transaction<'_>,
    row: &TransitionRow,
    now: UnixTimestamp,
) -> Result<()> {
    set_phase(
        transaction,
        &row.transition_id,
        WriterTransitionPhase::ClaimsFenced,
        now,
    )?;
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'transitioning',
                 health_reason = 'transition_in_progress',
                 transition_phase = 'claims_fenced',
                 active_transition_id = ?1,
                 claim_fence_epoch = ?2,
                 updated_at_seconds = ?3
             WHERE state_key = 'default'",
            params![
                row.transition_id.as_str(),
                i64::try_from(row.claim_fence_epoch)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn commit_target_contract(
    transaction: &Transaction<'_>,
    row: &TransitionRow,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    if running_count(transaction)? > 0 {
        return Err(MetaStoreError::invalid_transition());
    }
    retire_and_rebuild_queued_intents(transaction, desired, now)?;
    transaction
        .execute(
            "UPDATE migration_rebuild_contract_state
             SET active_contract_id = ?1, updated_at_seconds = ?2
             WHERE state_key = 'default'",
            params![desired.id().as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    set_phase(
        transaction,
        &row.transition_id,
        WriterTransitionPhase::TargetCommitted,
        now,
    )?;
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'transitioning',
                 health_reason = 'transition_in_progress',
                 transition_phase = 'target_committed',
                 committed_contract_id = ?1,
                 desired_contract_id = ?1,
                 updated_at_seconds = ?2
             WHERE state_key = 'default'",
            params![desired.id().as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn retire_and_rebuild_queued_intents(
    transaction: &Transaction<'_>,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT task.id, task.root_path, task.updated_at_seconds
             FROM import_task AS task
             JOIN import_task_contract_binding AS binding
               ON binding.import_task_id = task.id
             WHERE task.status IN (?1, ?2)
               AND binding.processing_contract_id != ?3
               AND NOT EXISTS (
                   SELECT 1 FROM import_task_cancellation AS cancellation
                   WHERE cancellation.import_task_id = task.id
               )
             ORDER BY task.root_path, task.queued_at_seconds, task.rowid",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![
            import_task_status_to_storage(ImportTaskStatus::Queued),
            import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            desired.id().as_str(),
        ])
        .map_err(MetaStoreError::storage)?;
    let mut pending: Vec<(String, String, i64)> = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        pending.push((
            row.get(0).map_err(MetaStoreError::storage)?,
            row.get(1).map_err(MetaStoreError::storage)?,
            row.get(2).map_err(MetaStoreError::storage)?,
        ));
    }
    drop(rows);
    drop(statement);

    // Stage every root replacement before cancelling any old intent. Any root
    // that cannot be rebuilt rolls the whole transition back.
    let mut root_replacement: std::collections::BTreeMap<
        String,
        (ImportTask, crate::ImportScanScope),
    > = std::collections::BTreeMap::new();
    for (_task_id, root_path, _updated_at) in &pending {
        if root_replacement.contains_key(root_path) {
            continue;
        }
        let new_task_id = ImportTaskId::from_non_secret_parts(&[
            "writer-contract-rebuild",
            root_path.as_str(),
            desired.id().as_str(),
            &now.as_unix_seconds().to_string(),
        ]);
        let scope = authorized_root_task_scope(transaction, root_path, &new_task_id, now).map_err(
            |error| {
                if error.class() == crate::MetaStoreErrorClass::NotFound {
                    MetaStoreError::invalid_transition()
                } else {
                    error
                }
            },
        )?;
        root_replacement.insert(
            root_path.clone(),
            (
                ImportTask {
                    id: new_task_id,
                    root_path: root_path.clone(),
                    status: ImportTaskStatus::Queued,
                    queued_at: now,
                    started_at: None,
                    finished_at: None,
                    updated_at: now,
                },
                scope,
            ),
        );
    }

    for (old_task_id, _root_path, updated_at) in &pending {
        let cancel_at = now.as_unix_seconds().max(*updated_at);
        transaction
            .execute(
                "INSERT OR IGNORE INTO import_task_cancellation (
                     import_task_id, requested_at_seconds
                 ) VALUES (?1, ?2)",
                params![old_task_id.as_str(), cancel_at],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "UPDATE import_task
                 SET updated_at_seconds = MAX(updated_at_seconds, ?1)
                 WHERE id = ?2",
                params![cancel_at, old_task_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        // Scheduled PDF jobs bound to the retired task must become claimable again.
        transaction
            .execute(
                "UPDATE pdf_reprocess_job
                 SET state = 'queued',
                     scheduled_task_id = NULL,
                     processing_contract_id = ?1,
                     updated_at_seconds = MAX(updated_at_seconds, ?2)
                 WHERE scheduled_task_id = ?3
                   AND state = 'scheduled'",
                params![
                    desired.id().as_str(),
                    now.as_unix_seconds(),
                    old_task_id.as_str()
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }

    for (task, scope) in root_replacement.values() {
        crate::insert_import_task_with_scan_scope_in_connection(transaction, task, scope, desired)?;
    }
    Ok(())
}

fn materialize_campaign(
    transaction: &Transaction<'_>,
    row: &TransitionRow,
    strategy: ContractTransitionStrategy,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    let Some(domain) = campaign_domain_for(strategy) else {
        return Ok(());
    };
    if domain == "unsupported" {
        return Ok(());
    }
    let campaign_id = new_digest_id(&[
        b"reprocessing-campaign",
        row.transition_id.as_bytes(),
        domain.as_bytes(),
    ]);
    transaction
        .execute(
            "INSERT OR IGNORE INTO reprocessing_campaign (
                campaign_id, transition_id, target_contract_id, affected_domain,
                state, created_at_seconds, updated_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?5)",
            params![
                campaign_id.as_str(),
                row.transition_id.as_str(),
                desired.id().as_str(),
                domain,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    transaction
        .execute(
            "UPDATE writer_contract_transition
             SET campaign_id = ?1, updated_at_seconds = ?2
             WHERE transition_id = ?3",
            params![
                campaign_id.as_str(),
                now.as_unix_seconds(),
                row.transition_id.as_str()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if matches!(
        strategy,
        ContractTransitionStrategy::PdfRootRescan | ContractTransitionStrategy::DerivedRebuild
    ) {
        transaction
            .execute(
                "UPDATE pdf_reprocess_job
                 SET processing_contract_id = ?1,
                     campaign_id = ?2,
                     updated_at_seconds = MAX(updated_at_seconds, ?3)
                 WHERE state IN ('queued', 'scheduled')",
                params![
                    desired.id().as_str(),
                    campaign_id.as_str(),
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    Ok(())
}

fn record_blocked_attempt(
    transaction: &Transaction<'_>,
    desired: &ImportProcessingContract,
    product_version: &str,
    desired_schema_version: u32,
    now: UnixTimestamp,
) -> Result<()> {
    let transition_id = new_digest_id(&[
        b"writer-contract-transition-blocked",
        desired.id().as_str().as_bytes(),
        &now.as_unix_seconds().to_le_bytes(),
    ]);
    let source_id = active_contract(transaction)?.map(|c| c.id().as_str().to_string());
    let (generation, visible_epoch) = projection_identity(transaction)?;
    let claim_fence_epoch = next_claim_fence_epoch(transaction)?;
    transaction
        .execute(
            "INSERT INTO writer_contract_transition (
                transition_id, source_contract_id, target_contract_id,
                desired_product_version, desired_schema_version,
                source_generation, source_visible_epoch, phase, attempt,
                claim_fence_epoch, running_task_count, queued_task_count,
                scheduled_task_count, failure_class, retryable,
                created_at_seconds, updated_at_seconds
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'observed', 1, ?8, ?9, 0, 0,
                'blocked_by_running_owner', 1, ?10, ?10
             )",
            params![
                transition_id.as_str(),
                source_id.as_deref(),
                desired.id().as_str(),
                product_version,
                i64::from(desired_schema_version),
                generation.as_deref(),
                visible_epoch,
                i64::try_from(claim_fence_epoch).map_err(|_| MetaStoreError::storage_invariant())?,
                i64::try_from(running_count(transaction)?)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    set_authority_transitioning(
        transaction,
        &transition_id,
        WriterTransitionPhase::Observed,
        claim_fence_epoch,
        source_id.as_deref(),
        desired.id().as_str(),
        now,
    )?;
    Ok(())
}

fn mark_transition_failure(
    transaction: &Transaction<'_>,
    transition_id: &str,
    failure_class: &str,
    retryable: bool,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_contract_transition
             SET failure_class = ?1,
                 retryable = ?2,
                 updated_at_seconds = ?3
             WHERE transition_id = ?4",
            params![
                failure_class,
                i64::from(retryable),
                now.as_unix_seconds(),
                transition_id
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_phase(
    transaction: &Transaction<'_>,
    transition_id: &str,
    phase: WriterTransitionPhase,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_contract_transition
             SET phase = ?1, updated_at_seconds = ?2
             WHERE transition_id = ?3",
            params![phase.as_str(), now.as_unix_seconds(), transition_id],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_authority_phase(
    transaction: &Transaction<'_>,
    phase: WriterTransitionPhase,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET transition_phase = ?1, updated_at_seconds = ?2
             WHERE state_key = 'default'",
            params![phase.as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_authority_transitioning(
    transaction: &Transaction<'_>,
    transition_id: &str,
    phase: WriterTransitionPhase,
    claim_fence_epoch: u64,
    committed_contract_id: Option<&str>,
    desired_contract_id: &str,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'transitioning',
                 health_reason = 'transition_in_progress',
                 transition_phase = ?1,
                 active_transition_id = ?2,
                 claim_fence_epoch = ?3,
                 committed_contract_id = COALESCE(?4, committed_contract_id),
                 desired_contract_id = ?5,
                 updated_at_seconds = ?6
             WHERE state_key = 'default'",
            params![
                phase.as_str(),
                transition_id,
                i64::try_from(claim_fence_epoch).map_err(|_| MetaStoreError::storage_invariant())?,
                committed_contract_id,
                desired_contract_id,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_authority_ready_after_transition(
    transaction: &Transaction<'_>,
    contract_id: &ImportProcessingContractId,
    completed_transition_id: &str,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'ready',
                 health_reason = NULL,
                 transition_phase = 'writer_ready',
                 active_transition_id = NULL,
                 last_completed_transition_id = ?1,
                 committed_contract_id = ?2,
                 desired_contract_id = ?2,
                 updated_at_seconds = ?3
             WHERE state_key = 'default'",
            params![
                completed_transition_id,
                contract_id.as_str(),
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn sync_authority_ready(
    transaction: &Transaction<'_>,
    contract_id: &ImportProcessingContractId,
    now: UnixTimestamp,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'ready',
                 health_reason = NULL,
                 transition_phase = COALESCE(transition_phase, 'writer_ready'),
                 active_transition_id = NULL,
                 committed_contract_id = ?1,
                 desired_contract_id = ?1,
                 updated_at_seconds = MAX(updated_at_seconds, ?2)
             WHERE state_key = 'default'",
            params![contract_id.as_str(), now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn read_authority_snapshot(connection: &Connection) -> Result<WriterAuthoritySnapshot> {
    connection
        .query_row(
            "SELECT health_state, health_reason, transition_phase, active_transition_id,
                    last_completed_transition_id, claim_fence_epoch,
                    committed_contract_id, desired_contract_id
             FROM writer_authority_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)
        .and_then(
            |(
                health_state,
                health_reason,
                transition_phase,
                active_transition_id,
                last_completed_transition_id,
                claim_fence_epoch,
                committed_contract_id,
                desired_contract_id,
            )| {
                Ok(WriterAuthoritySnapshot {
                    health_state: WriterAuthorityHealthState::parse(&health_state)
                        .ok_or_else(MetaStoreError::storage_invariant)?,
                    health_reason,
                    transition_phase: match transition_phase.as_deref() {
                        None => None,
                        Some(value) => Some(
                            WriterTransitionPhase::parse(value)
                                .ok_or_else(MetaStoreError::storage_invariant)?,
                        ),
                    },
                    active_transition_id,
                    last_completed_transition_id,
                    claim_fence_epoch: u64::try_from(claim_fence_epoch)
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    committed_contract_id: committed_contract_id
                        .as_deref()
                        .map(ImportProcessingContractId::from_str)
                        .transpose()?,
                    desired_contract_id: desired_contract_id
                        .as_deref()
                        .map(ImportProcessingContractId::from_str)
                        .transpose()?,
                })
            },
        )
}

fn active_contract(connection: &Connection) -> Result<Option<ImportProcessingContract>> {
    let active_id = if connection
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'writer_authority_state'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        == Some(1)
    {
        connection
            .query_row(
                "SELECT COALESCE(writer.committed_contract_id, legacy.active_contract_id)
                 FROM writer_authority_state AS writer
                 CROSS JOIN migration_rebuild_contract_state AS legacy
                 WHERE writer.state_key = 'default'
                   AND legacy.state_key = 'default'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(MetaStoreError::storage)?
    } else {
        connection
            .query_row(
                "SELECT active_contract_id FROM migration_rebuild_contract_state
                 WHERE state_key = 'default'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(MetaStoreError::storage)?
    };
    let Some(active_id) = active_id else {
        return Ok(None);
    };
    let id = ImportProcessingContractId::from_str(&active_id)?;
    crate::import_processing_store::import_processing_contract_in_connection(connection, &id)
}

fn projection_is_ready_with_generation(connection: &Connection) -> Result<bool> {
    let state = connection
        .query_row(
            "SELECT service_state, generation
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(MetaStoreError::storage)?;
    Ok(state.0 == "ready" && state.1.is_some())
}

fn projection_identity(connection: &Connection) -> Result<(Option<String>, i64)> {
    connection
        .query_row(
            "SELECT generation, COALESCE(visible_epoch, 0)
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(MetaStoreError::storage)
}

fn running_count(connection: &Connection) -> Result<u64> {
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM import_task WHERE status = ?1",
            params![import_task_status_to_storage(ImportTaskStatus::Running)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u64::try_from(count).map_err(|_| MetaStoreError::storage_invariant())
}

fn queued_count(connection: &Connection) -> Result<u64> {
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM import_task WHERE status IN (?1, ?2)",
            params![
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u64::try_from(count).map_err(|_| MetaStoreError::storage_invariant())
}

fn next_claim_fence_epoch(connection: &Connection) -> Result<u64> {
    let current = connection
        .query_row(
            "SELECT claim_fence_epoch FROM writer_authority_state WHERE state_key = 'default'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u64::try_from(current)
        .map_err(|_| MetaStoreError::storage_invariant())
        .map(|epoch| epoch.saturating_add(1))
}

fn active_transition_row(connection: &Connection) -> Result<Option<TransitionRow>> {
    let active_id = connection
        .query_row(
            "SELECT active_transition_id FROM writer_authority_state WHERE state_key = 'default'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let Some(active_id) = active_id else {
        return Ok(None);
    };
    let raw = connection
        .query_row(
            "SELECT transition_id, source_contract_id, target_contract_id, phase,
                    claim_fence_epoch, attempt
             FROM writer_contract_transition WHERE transition_id = ?1",
            params![active_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let Some((
        transition_id,
        source_contract_id,
        target_contract_id,
        phase,
        claim_fence_epoch,
        attempt,
    )) = raw
    else {
        return Ok(None);
    };
    Ok(Some(TransitionRow {
        transition_id,
        source_contract_id,
        target_contract_id,
        phase: WriterTransitionPhase::parse(&phase)
            .ok_or_else(MetaStoreError::storage_invariant)?,
        claim_fence_epoch: u64::try_from(claim_fence_epoch)
            .map_err(|_| MetaStoreError::storage_invariant())?,
        attempt: u32::try_from(attempt).map_err(|_| MetaStoreError::storage_invariant())?,
    }))
}

fn new_digest_id(parts: &[&[u8]]) -> String {
    let mut joined = Vec::new();
    for part in parts {
        joined.extend_from_slice(part);
        joined.push(0);
    }
    ContentDigest::from_bytes(&joined).as_str().to_string()
}

struct TransitionRow {
    transition_id: String,
    source_contract_id: Option<String>,
    target_contract_id: String,
    phase: WriterTransitionPhase,
    claim_fence_epoch: u64,
    attempt: u32,
}

// Silence unused receipt type until public projection expands.
#[allow(dead_code)]
fn receipt_from_row(row: &TransitionRow, campaign_id: Option<String>) -> WriterTransitionReceipt {
    WriterTransitionReceipt {
        transition_id: row.transition_id.clone(),
        phase: row.phase,
        attempt: row.attempt,
        claim_fence_epoch: row.claim_fence_epoch,
        failure_class: None,
        retryable: false,
        campaign_id,
    }
}

#[cfg(test)]
#[path = "writer_transition_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "writer_transition_crash_tests.rs"]
mod crash_tests;
