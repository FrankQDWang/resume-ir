//! Migration-rebuild publication, durable retry accounting, and failed-attempt cleanup.

use std::collections::BTreeSet;

use meta_store::{
    DocumentStatus, ImportProcessingContract, MigrationRebuildBarrierToken,
    MigrationRebuildPublicationAttemptAcquire, MigrationRebuildPublicationErrorClass,
    MigrationRebuildPublicationFailure, OwnedMetaStore, SearchProjectionServiceState,
    SearchPublicationSession, SearchRepairReason, UnixTimestamp,
};

use super::artifact_maintenance::collect_obsolete_artifacts_best_effort;
use super::{SearchArtifactRecoverySummary, RECOVERY_PUBLICATION_LIMIT};
use crate::search_artifacts::{
    migration_index_documents_from_exact_projection, publish_migration_rebuild_search_artifacts,
};
use crate::search_publication::SearchPublicationTransactionOutcome;
use crate::search_publication_commit::decide_migration_rebuild_search_publication;
use crate::search_publication_failure::{
    abandon_and_retire_search_publication, replay_pending_search_publication_retirements,
    FailedGenerationArtifacts,
};
use crate::{
    current_timestamp_or, ImportPipelineError, ImportPipelineErrorClass, ImportPipelineErrorKind,
    PipelineRunControl, Result, SearchPublicationVectorization,
};

pub fn finalize_migration_rebuild(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    contract: &ImportProcessingContract,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
) -> Result<SearchArtifactRecoverySummary> {
    finalize_migration_rebuild_with_fault(
        store,
        now,
        contract,
        vectorization,
        control,
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
    CancelledPublication,
    #[cfg(test)]
    HoldBeforeFullText(&'static MigrationPublicationTestGate),
    #[cfg(test)]
    HoldBeforeFullTextForCancellation(&'static MigrationPublicationTestGate),
    #[cfg(test)]
    SignalBeforePublicationSession(&'static std::sync::Barrier),
    #[cfg(test)]
    CommitSuperseded(&'static MigrationPublicationCommitObserver),
    #[cfg(test)]
    CommitSupersededWithRetirementLease(&'static MigrationPublicationCommitObserver),
}

#[cfg(test)]
pub(crate) struct MigrationPublicationTestGate {
    entered: std::sync::Barrier,
    release: std::sync::Barrier,
}

#[cfg(test)]
pub(crate) struct MigrationPublicationCommitObserver {
    generation: std::sync::Mutex<Option<String>>,
    fulltext_reader: std::sync::Mutex<Option<index_fulltext::FullTextIndex>>,
}

pub(crate) fn finalize_migration_rebuild_with_fault(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    contract: &ImportProcessingContract,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
    fault: MigrationPublicationFault,
) -> Result<SearchArtifactRecoverySummary> {
    control.ensure_running()?;
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    let migration_rebuild_active = state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::MigrationRebuild)
        && state.generation.is_none();
    if !migration_rebuild_active
        && state.service_state != SearchProjectionServiceState::RepairBlocked
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
    control.ensure_running()?;
    let replayed = replay_pending_search_publication_retirements(&publication_session, now)?;
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state != SearchProjectionServiceState::Repairing
        || state.repair_reason != Some(SearchRepairReason::MigrationRebuild)
        || state.generation.is_some()
    {
        return Ok(SearchArtifactRecoverySummary {
            interrupted_publications_abandoned: replayed,
            ..SearchArtifactRecoverySummary::default()
        });
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
    let recovered_publications = replayed + interrupted.len();

    let Some(barrier_token) = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(SearchArtifactRecoverySummary::default());
    };
    control.ensure_running()?;
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
    let result = control.ensure_running().and_then(|()| {
        run_migration_rebuild_publication_attempt(
            &publication_session,
            now,
            contract,
            vectorization,
            &barrier_token,
            recovered_publications,
            control,
            fault,
        )
    });
    match result {
        Ok(MigrationPublicationAttemptOutcome::Applied(mut summary)) => {
            if control.shutdown_requested() {
                drop(retained_publication_lease);
                return Ok(summary);
            }
            collect_obsolete_artifacts_best_effort(
                &publication_session,
                &mut summary,
                Some(control),
            )?;
            drop(retained_publication_lease);
            Ok(summary)
        }
        Ok(MigrationPublicationAttemptOutcome::Superseded(summary)) => {
            publication_session
                .abandon_migration_rebuild_publication_attempt(&attempt)
                .map_err(ImportPipelineError::store)?;
            drop(retained_publication_lease);
            Ok(summary)
        }
        Err(error)
            if matches!(
                error.class(),
                ImportPipelineErrorClass::Cancelled | ImportPipelineErrorClass::Interrupted
            ) =>
        {
            // Cancellation is lifecycle control, not a failed publication.
            // This migration path's only cancellation source is its
            // PipelineRunControl, so Cancelled cannot mean a durable user
            // cancellation here.
            // Staged search publication journals are abandoned by the
            // publication boundary that created them. Do not perform the
            // unbounded artifact GC path or consume durable retry budget while
            // the supervisor is trying to stop this generation.
            publication_session
                .abandon_migration_rebuild_publication_attempt(&attempt)
                .map_err(ImportPipelineError::store)?;
            drop(retained_publication_lease);
            Err(error)
        }
        Err(error) => {
            let failure = migration_publication_failure(&error);
            let failed_at = migration_attempt_finished_at(fault, now);
            publication_session
                .finish_migration_rebuild_publication_attempt_failure(&attempt, failure, failed_at)
                .map_err(ImportPipelineError::store)?;
            drop(retained_publication_lease);
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
    recovered_publications: usize,
    control: &PipelineRunControl,
    _fault: MigrationPublicationFault,
) -> Result<MigrationPublicationAttemptOutcome> {
    control.ensure_running()?;
    let store = publication_session.owned_store();

    let projection_rows = store
        .migration_rebuild_projection_rows(barrier_token)
        .map_err(ImportPipelineError::store)?;
    control.ensure_running()?;
    #[cfg(test)]
    match _fault {
        MigrationPublicationFault::RetryableFullText
        | MigrationPublicationFault::RetryableFullTextFinishedAt(_)
        | MigrationPublicationFault::SignalBeforePublicationSession(_) => {
            return Err(ImportPipelineError::index_io());
        }
        MigrationPublicationFault::CancelledPublication => {
            return Err(ImportPipelineError::cancelled());
        }
        MigrationPublicationFault::HoldBeforeFullText(gate) => {
            gate.entered.wait();
            gate.release.wait();
            return Err(ImportPipelineError::index_io());
        }
        MigrationPublicationFault::HoldBeforeFullTextForCancellation(gate) => {
            gate.entered.wait();
            gate.release.wait();
            control.ensure_running()?;
        }
        MigrationPublicationFault::CommitSuperseded(_)
        | MigrationPublicationFault::CommitSupersededWithRetirementLease(_)
        | MigrationPublicationFault::None => {}
    }
    let staged = migration_index_documents_from_exact_projection(
        projection_rows,
        Some(&|| control.ensure_running()),
    )?;
    control.ensure_running()?;
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

    let publication = publish_migration_rebuild_search_artifacts(
        publication_session,
        now,
        contract.classifier_epoch(),
        &pending_document_ids,
        index_documents,
        vectorization,
        Some(&|| control.ensure_running()),
        |publication| {
            #[cfg(test)]
            if let MigrationPublicationFault::CommitSuperseded(observer)
            | MigrationPublicationFault::CommitSupersededWithRetirementLease(observer) = _fault
            {
                *observer.generation.lock().unwrap() = Some(publication.generation().to_string());
                if matches!(
                    _fault,
                    MigrationPublicationFault::CommitSupersededWithRetirementLease(_)
                ) {
                    let fulltext_root = publication
                        .publication_session()
                        .canonical_data_dir()
                        .join("search-index");
                    let lease = index_fulltext::SnapshotReadLease::acquire(&fulltext_root)
                        .unwrap()
                        .expect("prepared full-text root must be readable");
                    let reader = index_fulltext::FullTextIndex::open_snapshot_with_lease(
                        &fulltext_root,
                        publication.generation(),
                        lease,
                    )
                    .unwrap()
                    .expect("prepared full-text generation must be readable");
                    *observer.fulltext_reader.lock().unwrap() = Some(reader);
                }
                return crate::search_publication_commit::decide_migration_rebuild_search_publication_with_outcome_for_test(
                    publication,
                    now,
                    &documents,
                    meta_store::SearchPublicationOutcome::Superseded,
                );
            }
            decide_migration_rebuild_search_publication(publication, now, &documents, barrier_token)
        },
    )?;
    let SearchPublicationTransactionOutcome::Committed(committed) = publication else {
        return Ok(MigrationPublicationAttemptOutcome::Superseded(
            SearchArtifactRecoverySummary {
                interrupted_publications_abandoned: recovered_publications,
                ..SearchArtifactRecoverySummary::default()
            },
        ));
    };
    committed.release();

    Ok(MigrationPublicationAttemptOutcome::Applied(
        SearchArtifactRecoverySummary {
            interrupted_publications_abandoned: recovered_publications,
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
        MigrationPublicationFault::CancelledPublication => return not_before,
        MigrationPublicationFault::RetryableFullTextFinishedAt(finished_at) => {
            return UnixTimestamp::from_unix_seconds(
                finished_at
                    .as_unix_seconds()
                    .max(not_before.as_unix_seconds()),
            );
        }
        MigrationPublicationFault::HoldBeforeFullText(_)
        | MigrationPublicationFault::HoldBeforeFullTextForCancellation(_)
        | MigrationPublicationFault::SignalBeforePublicationSession(_)
        | MigrationPublicationFault::CommitSuperseded(_)
        | MigrationPublicationFault::CommitSupersededWithRetirementLease(_) => return not_before,
        MigrationPublicationFault::None => {}
    }
    current_timestamp_or(not_before)
}

fn migration_publication_failure(
    error: &ImportPipelineError,
) -> MigrationRebuildPublicationFailure {
    let error_class = match error.kind {
        ImportPipelineErrorKind::VectorArtifactRetirement => {
            MigrationRebuildPublicationErrorClass::Vector
        }
        ImportPipelineErrorKind::FullTextArtifactRetirement => {
            MigrationRebuildPublicationErrorClass::FullText
        }
        _ => match error.class() {
            ImportPipelineErrorClass::VectorContract
            | ImportPipelineErrorClass::VectorStorage
            | ImportPipelineErrorClass::EmbeddingRuntime => {
                MigrationRebuildPublicationErrorClass::Vector
            }
            ImportPipelineErrorClass::Metadata
            | ImportPipelineErrorClass::MetadataInvariant
            | ImportPipelineErrorClass::Repairing => {
                MigrationRebuildPublicationErrorClass::Metadata
            }
            ImportPipelineErrorClass::FullText
            | ImportPipelineErrorClass::Cancelled
            | ImportPipelineErrorClass::Interrupted
            | ImportPipelineErrorClass::ArtifactRetirement
            | ImportPipelineErrorClass::SourceUnavailable
            | ImportPipelineErrorClass::Scan
            | ImportPipelineErrorClass::Privacy
            | ImportPipelineErrorClass::Parser => MigrationRebuildPublicationErrorClass::FullText,
        },
    };
    if error.is_retryable() {
        MigrationRebuildPublicationFailure::Retryable(error_class)
    } else {
        MigrationRebuildPublicationFailure::Terminal(error_class)
    }
}

#[cfg(test)]
#[path = "migration_publication_tests.rs"]
mod tests;
