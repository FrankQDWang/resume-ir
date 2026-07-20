//! Normal startup reconciliation for already-published search projections.

use meta_store::{
    ArtifactRepairAttempt, ArtifactRepairAttemptAcquire, ArtifactRepairAttemptErrorKind,
    ArtifactRepairAttemptFailure, ArtifactRepairKey, OwnedMetaStore, SearchProjectionServiceState,
    SearchProjectionTransitionOutcome, SearchPublicationSession, SearchRepairReason, UnixTimestamp,
};

use super::artifact_maintenance::{
    active_artifacts_are_usable, collect_obsolete_artifacts_best_effort,
    ActiveArtifactValidationDepth,
};
use super::{SearchArtifactRecoverySummary, RECOVERY_PUBLICATION_LIMIT};
use crate::search_artifacts::publish_rebuilt_search_artifacts_from_base;
use crate::search_publication::load_search_publication_base;
use crate::search_publication_commit::decide_search_publication;
use crate::search_publication_failure::{
    abandon_and_retire_search_publication,
    replay_pending_search_publication_retirements_classified, FailedGenerationArtifacts,
    PendingRetirementReplay,
};
use crate::{
    current_timestamp_or, ImportPipelineError, ImportPipelineErrorClass, ImportPipelineErrorKind,
    PipelineRunControl, Result, SearchPublicationVectorization,
};

pub fn reconcile_search_artifacts(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
) -> Result<SearchArtifactRecoverySummary> {
    reconcile_search_artifacts_with_validation(
        store,
        now,
        vectorization,
        control,
        ActiveArtifactValidationDepth::ManifestOnly,
        ReconciliationMode::ResidentBestEffort,
    )
}

/// Reconciles artifacts before an offline mutation that will consume the
/// current immutable payloads as its publication base.
pub fn reconcile_search_artifacts_for_offline_mutation(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
) -> Result<SearchArtifactRecoverySummary> {
    reconcile_search_artifacts_with_validation(
        store,
        now,
        vectorization,
        control,
        ActiveArtifactValidationDepth::OpenPayloads,
        ReconciliationMode::OfflineMutationRequiresReady,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconciliationMode {
    ResidentBestEffort,
    OfflineMutationRequiresReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconciliationDeferral {
    RepairBlocked,
    PublicationOwnershipUnavailable,
    NoPublishedGeneration,
    HeadSuperseded,
    RepairContextSuperseded,
    AttemptInProgress,
    AttemptNotDue,
    AttemptRepairBlocked,
    AttemptSuperseded,
    RepairFailed,
    IncompleteReadyHead,
    ShutdownAfterPublication,
}

fn reconcile_search_artifacts_with_validation(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
    validation_depth: ActiveArtifactValidationDepth,
    mode: ReconciliationMode,
) -> Result<SearchArtifactRecoverySummary> {
    control.ensure_running()?;
    let mut publication_session = match store.try_acquire_search_publication_session() {
        Ok(session) => session,
        Err(error)
            if error.class() == meta_store::MetaStoreErrorClass::MigrationOwnershipRequired =>
        {
            return defer_reconciliation(
                mode,
                SearchArtifactRecoverySummary::default(),
                ReconciliationDeferral::PublicationOwnershipUnavailable,
            );
        }
        Err(error) => return Err(ImportPipelineError::store(error)),
    };
    control.ensure_running()?;
    let replayed = match replay_pending_search_publication_retirements_classified(
        &publication_session,
        now,
    )? {
        PendingRetirementReplay::Replayed(replayed) => replayed,
        PendingRetirementReplay::CurrentHeadBlocked(error) => {
            return match mode {
                ReconciliationMode::ResidentBestEffort => {
                    Ok(SearchArtifactRecoverySummary::default())
                }
                ReconciliationMode::OfflineMutationRequiresReady => Err(error),
            };
        }
    };
    let mut summary = SearchArtifactRecoverySummary {
        interrupted_publications_abandoned: replayed,
        ..SearchArtifactRecoverySummary::default()
    };
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::RepairBlocked {
        return defer_reconciliation(mode, summary, ReconciliationDeferral::RepairBlocked);
    }
    let interrupted = store
        .interrupted_search_publications(RECOVERY_PUBLICATION_LIMIT)
        .map_err(ImportPipelineError::store)?;
    for publication in &interrupted {
        control.ensure_running()?;
        abandon_and_retire_search_publication(
            &publication_session,
            &publication.generation,
            now,
            FailedGenerationArtifacts::both_may_exist(),
        )?;
    }
    summary.interrupted_publications_abandoned += interrupted.len();

    if state.generation.is_none() {
        if state.service_state == SearchProjectionServiceState::Ready {
            return Err(ImportPipelineError::store_invariant());
        }
        return defer_reconciliation(mode, summary, ReconciliationDeferral::NoPublishedGeneration);
    }

    let artifact_repair_in_progress = state.service_state
        == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::ArtifactUnavailable);
    let artifact_validation = if artifact_repair_in_progress {
        Ok(false)
    } else {
        active_artifacts_are_usable(&publication_session, vectorization, validation_depth)
    };
    let artifact_rebuild_required = artifact_repair_in_progress
        || match &artifact_validation {
            Ok(artifacts_usable) => !artifacts_usable,
            Err(_) => true,
        };
    if artifact_rebuild_required {
        let expected_generation = state
            .generation
            .as_deref()
            .ok_or_else(ImportPipelineError::store_invariant)?;
        let expected_visible_epoch = state.visible_epoch;
        control.ensure_running()?;
        if store
            .begin_artifact_repair(expected_generation, expected_visible_epoch, now)
            .map_err(ImportPipelineError::store)?
            == SearchProjectionTransitionOutcome::Superseded
        {
            return defer_reconciliation(mode, summary, ReconciliationDeferral::HeadSuperseded);
        }
        let context = store
            .artifact_repair_context()
            .map_err(ImportPipelineError::store)?
            .ok_or_else(ImportPipelineError::store_invariant)?;
        if context.generation != expected_generation
            || context.visible_epoch != expected_visible_epoch
        {
            return defer_reconciliation(
                mode,
                summary,
                ReconciliationDeferral::RepairContextSuperseded,
            );
        }
        let repair_key = ArtifactRepairKey::new(
            context.generation,
            context.publication_fingerprint,
            context.visible_epoch,
        );
        let attempt = match publication_session
            .acquire_artifact_repair_attempt(&repair_key, now)
            .map_err(ImportPipelineError::store)?
        {
            ArtifactRepairAttemptAcquire::Started(attempt) => attempt,
            ArtifactRepairAttemptAcquire::InProgress => {
                return defer_reconciliation(
                    mode,
                    summary,
                    ReconciliationDeferral::AttemptInProgress,
                );
            }
            ArtifactRepairAttemptAcquire::NotDue => {
                return defer_reconciliation(mode, summary, ReconciliationDeferral::AttemptNotDue);
            }
            ArtifactRepairAttemptAcquire::RepairBlocked => {
                return defer_reconciliation(
                    mode,
                    summary,
                    ReconciliationDeferral::AttemptRepairBlocked,
                );
            }
            ArtifactRepairAttemptAcquire::Superseded => {
                return defer_reconciliation(
                    mode,
                    summary,
                    ReconciliationDeferral::AttemptSuperseded,
                );
            }
        };
        if let Err(error) = control.ensure_running() {
            publication_session
                .cancel_artifact_repair_attempt(&attempt)
                .map_err(ImportPipelineError::store)?;
            return Err(error);
        }
        let rebuild = match artifact_validation {
            Err(error) => Err(error),
            Ok(_) => (|| {
                let base = load_search_publication_base(store)?;
                if base.generation.as_deref() != Some(expected_generation)
                    || base.visible_epoch != expected_visible_epoch
                {
                    return Err(ImportPipelineError::store_invariant());
                }
                let classifier_epoch = base.classifier_epoch.clone();
                publish_rebuilt_search_artifacts_from_base(
                    &publication_session,
                    now,
                    &classifier_epoch,
                    &Default::default(),
                    Vec::new(),
                    base,
                    vectorization,
                    Some(&|| control.ensure_running()),
                    |publication| decide_search_publication(publication, now, &[]),
                )?
                .into_committed()
            })(),
        };
        let committed = match rebuild {
            Ok(committed) => committed,
            Err(error) => {
                let finished_at = current_timestamp_or(now);
                settle_artifact_rebuild_failure(
                    &mut publication_session,
                    &attempt,
                    finished_at,
                    error,
                )?;
                return defer_reconciliation(mode, summary, ReconciliationDeferral::RepairFailed);
            }
        };
        summary.active_generation_rebuilt = true;
        if control.shutdown_requested() {
            committed.release();
            return defer_reconciliation(
                mode,
                summary,
                ReconciliationDeferral::ShutdownAfterPublication,
            );
        }
        collect_obsolete_artifacts_best_effort(&publication_session, &mut summary, Some(control))?;
        committed.release();
        return finish_reconciliation(store, mode, summary);
    }

    control.ensure_running()?;
    collect_obsolete_artifacts_best_effort(&publication_session, &mut summary, Some(control))?;
    finish_reconciliation(store, mode, summary)
}

fn defer_reconciliation(
    mode: ReconciliationMode,
    summary: SearchArtifactRecoverySummary,
    reason: ReconciliationDeferral,
) -> Result<SearchArtifactRecoverySummary> {
    match (mode, reason) {
        (ReconciliationMode::ResidentBestEffort, _) => Ok(summary),
        (
            ReconciliationMode::OfflineMutationRequiresReady,
            ReconciliationDeferral::RepairBlocked
            | ReconciliationDeferral::PublicationOwnershipUnavailable
            | ReconciliationDeferral::NoPublishedGeneration
            | ReconciliationDeferral::HeadSuperseded
            | ReconciliationDeferral::RepairContextSuperseded
            | ReconciliationDeferral::AttemptInProgress
            | ReconciliationDeferral::AttemptNotDue
            | ReconciliationDeferral::AttemptRepairBlocked
            | ReconciliationDeferral::AttemptSuperseded
            | ReconciliationDeferral::RepairFailed
            | ReconciliationDeferral::IncompleteReadyHead
            | ReconciliationDeferral::ShutdownAfterPublication,
        ) => Err(ImportPipelineError::repairing()),
    }
}

fn finish_reconciliation(
    store: &OwnedMetaStore,
    mode: ReconciliationMode,
    summary: SearchArtifactRecoverySummary,
) -> Result<SearchArtifactRecoverySummary> {
    if mode == ReconciliationMode::ResidentBestEffort {
        return Ok(summary);
    }
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::Ready
        && state.repair_reason.is_none()
        && state.generation.is_some()
        && state.publication.is_some()
    {
        Ok(summary)
    } else {
        defer_reconciliation(mode, summary, ReconciliationDeferral::IncompleteReadyHead)
    }
}

/// Finalizes one exact durable attempt. Lifecycle cancellation restores the
/// previous retry state, retryable failures schedule bounded backoff, and a
/// terminal or exhausted failure blocks only the context-bound head.
fn settle_artifact_rebuild_failure(
    publication_session: &mut SearchPublicationSession,
    attempt: &ArtifactRepairAttempt,
    now: UnixTimestamp,
    error: ImportPipelineError,
) -> Result<()> {
    if matches!(
        error.class(),
        ImportPipelineErrorClass::Cancelled | ImportPipelineErrorClass::Interrupted
    ) {
        publication_session
            .cancel_artifact_repair_attempt(attempt)
            .map_err(ImportPipelineError::store)?;
        return Err(error);
    }
    let failure = if error.is_retryable() {
        ArtifactRepairAttemptFailure::Retryable(artifact_repair_error_kind(&error))
    } else {
        ArtifactRepairAttemptFailure::Terminal(artifact_repair_error_kind(&error))
    };
    let _outcome = publication_session
        .finish_artifact_repair_attempt_failure(attempt, failure, now)
        .map_err(ImportPipelineError::store)?;
    Ok(())
}

fn artifact_repair_error_kind(error: &ImportPipelineError) -> ArtifactRepairAttemptErrorKind {
    match error.kind {
        ImportPipelineErrorKind::FullTextPublicationBusy => {
            ArtifactRepairAttemptErrorKind::FullTextPublicationBusy
        }
        ImportPipelineErrorKind::VectorPublicationBusy => {
            ArtifactRepairAttemptErrorKind::VectorPublicationBusy
        }
        ImportPipelineErrorKind::VectorContract
        | ImportPipelineErrorKind::VectorStorage
        | ImportPipelineErrorKind::VectorArtifactRetirement
        | ImportPipelineErrorKind::EmbeddingRuntime => {
            ArtifactRepairAttemptErrorKind::VectorFailure
        }
        ImportPipelineErrorKind::Store
        | ImportPipelineErrorKind::StoreInvariant(_)
        | ImportPipelineErrorKind::Repairing => ArtifactRepairAttemptErrorKind::MetadataFailure,
        ImportPipelineErrorKind::Cancelled | ImportPipelineErrorKind::Interrupted => {
            ArtifactRepairAttemptErrorKind::Interrupted
        }
        ImportPipelineErrorKind::Index
        | ImportPipelineErrorKind::ArtifactRetirement
        | ImportPipelineErrorKind::FullTextArtifactRetirement
        | ImportPipelineErrorKind::Crawl(_)
        | ImportPipelineErrorKind::Privacy
        | ImportPipelineErrorKind::Parser => ArtifactRepairAttemptErrorKind::FullTextFailure,
    }
}

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;
