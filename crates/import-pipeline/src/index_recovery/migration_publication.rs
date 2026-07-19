//! Migration-rebuild publication, durable retry accounting, and failed-attempt cleanup.

use std::collections::BTreeSet;

use meta_store::{
    DocumentStatus, ImportProcessingContract, MigrationRebuildBarrierToken,
    MigrationRebuildPublicationAttempt, MigrationRebuildPublicationAttemptAcquire,
    MigrationRebuildPublicationErrorClass, MigrationRebuildPublicationFailure, OwnedMetaStore,
    SearchProjectionServiceState, SearchPublicationLease, SearchPublicationSession,
    SearchRepairReason, UnixTimestamp,
};

use super::artifact_maintenance::collect_obsolete_artifacts;
use super::{SearchArtifactRecoverySummary, RECOVERY_PUBLICATION_LIMIT};
use crate::search_artifacts::{
    migration_index_documents_from_exact_projection, write_migration_rebuild_search_artifacts,
};
use crate::search_publication::commit_migration_rebuild_search_publication;
use crate::{
    current_timestamp_or, ImportPipelineError, ImportPipelineErrorClass, Result,
    SearchPublicationVectorization,
};

pub fn finalize_migration_rebuild(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    contract: &ImportProcessingContract,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactRecoverySummary> {
    finalize_migration_rebuild_with_fault(
        store,
        now,
        contract,
        vectorization,
        MigrationPublicationFault::None,
    )
}

#[derive(Clone, Copy)]
pub(crate) enum MigrationPublicationFault {
    None,
    #[cfg(test)]
    RetryableFullText,
    #[cfg(test)]
    RetryableFullTextFinishedAt(UnixTimestamp),
    #[cfg(test)]
    HoldBeforeFullText(&'static MigrationPublicationTestGate),
    #[cfg(test)]
    SignalBeforePublicationSession(&'static std::sync::Barrier),
}

#[cfg(test)]
pub(crate) struct MigrationPublicationTestGate {
    entered: std::sync::Barrier,
    release: std::sync::Barrier,
}

pub(crate) fn finalize_migration_rebuild_with_fault(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    contract: &ImportProcessingContract,
    vectorization: &SearchPublicationVectorization,
    fault: MigrationPublicationFault,
) -> Result<SearchArtifactRecoverySummary> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state != SearchProjectionServiceState::Repairing
        || state.repair_reason != Some(SearchRepairReason::MigrationRebuild)
        || state.generation.is_some()
    {
        return Ok(SearchArtifactRecoverySummary::default());
    }

    // Publication ownership must precede attempt reservation. Otherwise a
    // second caller can advance the durable attempt row while the first caller
    // is still blocked on this lock, consuming retry budget without a second
    // publication attempt ever having run.
    #[cfg(test)]
    if let MigrationPublicationFault::SignalBeforePublicationSession(entered) = fault {
        entered.wait();
    }
    let mut publication_session = match store.try_acquire_search_publication_session() {
        Ok(session) => session,
        Err(error) => {
            if error.class() != meta_store::MetaStoreErrorClass::MigrationOwnershipRequired {
                let _ = store.block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now);
            }
            return Err(ImportPipelineError::store(error));
        }
    };
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state != SearchProjectionServiceState::Repairing
        || state.repair_reason != Some(SearchRepairReason::MigrationRebuild)
        || state.generation.is_some()
    {
        return Ok(SearchArtifactRecoverySummary::default());
    }

    let Some(barrier_token) = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(SearchArtifactRecoverySummary::default());
    };
    let attempt = match publication_session
        .acquire_migration_rebuild_publication_attempt(&barrier_token, now)
        .map_err(ImportPipelineError::store)?
    {
        MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        MigrationRebuildPublicationAttemptAcquire::InProgress
        | MigrationRebuildPublicationAttemptAcquire::NotDue
        | MigrationRebuildPublicationAttemptAcquire::RepairBlocked
        | MigrationRebuildPublicationAttemptAcquire::Superseded => {
            return Ok(SearchArtifactRecoverySummary::default())
        }
    };

    let retained_publication_lease = publication_session.retain();
    let result = run_migration_rebuild_publication_attempt(
        &publication_session,
        now,
        contract,
        vectorization,
        &barrier_token,
        fault,
    );
    match result {
        Ok(MigrationPublicationAttemptOutcome::Applied(mut summary)) => {
            collect_obsolete_artifacts(&publication_session, &mut summary)?;
            drop(retained_publication_lease);
            Ok(summary)
        }
        Ok(MigrationPublicationAttemptOutcome::Superseded(mut summary)) => {
            let cleanup_at = migration_attempt_finished_at(fault, now);
            let cleanup = cleanup_failed_migration_publication(
                &publication_session,
                cleanup_at,
                &mut summary,
                retained_publication_lease,
            );
            let FailedMigrationPublicationCleanup {
                publication_lease,
                error,
            } = cleanup;
            if let Some(error) = error {
                finish_terminal_migration_cleanup_failure(
                    &mut publication_session,
                    &attempt,
                    migration_attempt_finished_at(fault, cleanup_at),
                )?;
                drop(publication_lease);
                return Err(error);
            }
            publication_session
                .abandon_migration_rebuild_publication_attempt(&attempt)
                .map_err(ImportPipelineError::store)?;
            drop(publication_lease);
            Ok(summary)
        }
        Err(error) => {
            let failure = migration_publication_failure(&error);
            let mut summary = SearchArtifactRecoverySummary::default();
            let cleanup_at = migration_attempt_finished_at(fault, now);
            let cleanup = cleanup_failed_migration_publication(
                &publication_session,
                cleanup_at,
                &mut summary,
                retained_publication_lease,
            );
            let FailedMigrationPublicationCleanup {
                publication_lease,
                error: cleanup_error,
            } = cleanup;
            if let Some(cleanup_error) = cleanup_error {
                finish_terminal_migration_cleanup_failure(
                    &mut publication_session,
                    &attempt,
                    migration_attempt_finished_at(fault, cleanup_at),
                )?;
                drop(publication_lease);
                return Err(cleanup_error);
            }
            let failed_at = migration_attempt_finished_at(fault, cleanup_at);
            publication_session
                .finish_migration_rebuild_publication_attempt_failure(&attempt, failure, failed_at)
                .map_err(ImportPipelineError::store)?;
            drop(publication_lease);
            Err(error)
        }
    }
}

enum MigrationPublicationAttemptOutcome {
    Applied(SearchArtifactRecoverySummary),
    Superseded(SearchArtifactRecoverySummary),
}

#[allow(clippy::too_many_arguments)]
fn run_migration_rebuild_publication_attempt(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    contract: &ImportProcessingContract,
    vectorization: &SearchPublicationVectorization,
    barrier_token: &MigrationRebuildBarrierToken,
    _fault: MigrationPublicationFault,
) -> Result<MigrationPublicationAttemptOutcome> {
    let store = publication_session.owned_store();
    let interrupted = store
        .interrupted_search_publications(RECOVERY_PUBLICATION_LIMIT)
        .map_err(ImportPipelineError::store)?;
    for publication in &interrupted {
        publication_session
            .abandon_search_publication(&publication.generation, now)
            .map_err(ImportPipelineError::store)?;
    }

    let projection_rows = store
        .migration_rebuild_projection_rows(barrier_token)
        .map_err(ImportPipelineError::store)?;
    #[cfg(test)]
    match _fault {
        MigrationPublicationFault::RetryableFullText
        | MigrationPublicationFault::RetryableFullTextFinishedAt(_)
        | MigrationPublicationFault::SignalBeforePublicationSession(_) => {
            return Err(ImportPipelineError::index_io());
        }
        MigrationPublicationFault::HoldBeforeFullText(gate) => {
            gate.entered.wait();
            gate.release.wait();
            return Err(ImportPipelineError::index_io());
        }
        MigrationPublicationFault::None => {}
    }
    let staged = migration_index_documents_from_exact_projection(projection_rows)?;
    let pending_document_ids = staged
        .iter()
        .map(|(_, index_document)| index_document.doc_id.clone())
        .collect::<BTreeSet<_>>();
    let index_documents = staged
        .iter()
        .map(|(_, index_document)| index_document.clone())
        .collect::<Vec<_>>();
    let mut documents = staged
        .into_iter()
        .map(|(mut document, _)| {
            document.status = DocumentStatus::Searchable;
            document.updated_at = now;
            document
        })
        .collect::<Vec<_>>();
    documents.sort_by(|left, right| left.id.cmp(&right.id));

    let publication = write_migration_rebuild_search_artifacts(
        publication_session,
        now,
        contract.classifier_epoch(),
        &pending_document_ids,
        index_documents,
        vectorization,
    )?;
    let Some(committed) =
        commit_migration_rebuild_search_publication(now, publication, &documents, barrier_token)?
    else {
        return Ok(MigrationPublicationAttemptOutcome::Superseded(
            SearchArtifactRecoverySummary {
                interrupted_publications_abandoned: interrupted.len(),
                ..SearchArtifactRecoverySummary::default()
            },
        ));
    };
    committed.release();

    Ok(MigrationPublicationAttemptOutcome::Applied(
        SearchArtifactRecoverySummary {
            interrupted_publications_abandoned: interrupted.len(),
            active_generation_rebuilt: true,
            ..SearchArtifactRecoverySummary::default()
        },
    ))
}

fn migration_attempt_finished_at(
    _fault: MigrationPublicationFault,
    not_before: UnixTimestamp,
) -> UnixTimestamp {
    #[cfg(test)]
    match _fault {
        MigrationPublicationFault::RetryableFullText => return not_before,
        MigrationPublicationFault::RetryableFullTextFinishedAt(finished_at) => {
            return UnixTimestamp::from_unix_seconds(
                finished_at
                    .as_unix_seconds()
                    .max(not_before.as_unix_seconds()),
            );
        }
        MigrationPublicationFault::HoldBeforeFullText(_)
        | MigrationPublicationFault::SignalBeforePublicationSession(_) => return not_before,
        MigrationPublicationFault::None => {}
    }
    current_timestamp_or(not_before)
}

fn migration_publication_failure(
    error: &ImportPipelineError,
) -> MigrationRebuildPublicationFailure {
    let error_class = match error.class() {
        ImportPipelineErrorClass::VectorContract
        | ImportPipelineErrorClass::VectorStorage
        | ImportPipelineErrorClass::EmbeddingRuntime => {
            MigrationRebuildPublicationErrorClass::Vector
        }
        ImportPipelineErrorClass::Metadata
        | ImportPipelineErrorClass::MetadataInvariant
        | ImportPipelineErrorClass::Repairing => MigrationRebuildPublicationErrorClass::Metadata,
        ImportPipelineErrorClass::FullText
        | ImportPipelineErrorClass::Cancelled
        | ImportPipelineErrorClass::Interrupted
        | ImportPipelineErrorClass::ArtifactRetirement
        | ImportPipelineErrorClass::SourceUnavailable
        | ImportPipelineErrorClass::Scan
        | ImportPipelineErrorClass::Privacy
        | ImportPipelineErrorClass::Parser => MigrationRebuildPublicationErrorClass::FullText,
    };
    if error.is_retryable() {
        MigrationRebuildPublicationFailure::Retryable(error_class)
    } else {
        MigrationRebuildPublicationFailure::Terminal(error_class)
    }
}

struct FailedMigrationPublicationCleanup {
    publication_lease: SearchPublicationLease,
    error: Option<ImportPipelineError>,
}

fn cleanup_failed_migration_publication(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    summary: &mut SearchArtifactRecoverySummary,
    publication_lease: SearchPublicationLease,
) -> FailedMigrationPublicationCleanup {
    let store = publication_session.owned_store();
    // Observe and abandon journals only while holding the same namespace lock
    // used by every publisher. Reading first would let a waiting writer create
    // a fresh journal in the gap and have it mistaken for this failed attempt.
    let cleanup_result = (|| {
        let interrupted = store
            .interrupted_search_publications(RECOVERY_PUBLICATION_LIMIT)
            .map_err(ImportPipelineError::store)?;
        for publication in &interrupted {
            publication_session
                .abandon_search_publication(&publication.generation, now)
                .map_err(ImportPipelineError::store)?;
        }
        summary.interrupted_publications_abandoned += interrupted.len();
        collect_obsolete_artifacts(publication_session, summary)?;
        if summary.gc_partial || summary.gc_deferred {
            return Err(ImportPipelineError::index_io());
        }
        Ok(())
    })();
    FailedMigrationPublicationCleanup {
        publication_lease,
        error: cleanup_result.err(),
    }
}

fn finish_terminal_migration_cleanup_failure(
    publication_session: &mut SearchPublicationSession,
    attempt: &MigrationRebuildPublicationAttempt,
    failed_at: UnixTimestamp,
) -> Result<()> {
    publication_session
        .finish_migration_rebuild_publication_attempt_failure(
            attempt,
            MigrationRebuildPublicationFailure::Terminal(
                MigrationRebuildPublicationErrorClass::Cleanup,
            ),
            failed_at,
        )
        .map_err(ImportPipelineError::store)?;
    Ok(())
}

#[cfg(test)]
#[path = "migration_publication_tests.rs"]
mod tests;
