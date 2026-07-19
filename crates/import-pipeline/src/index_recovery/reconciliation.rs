//! Normal startup reconciliation for already-published search projections.

use meta_store::{
    OwnedMetaStore, SearchProjectionServiceState, SearchProjectionTransitionOutcome,
    SearchRepairReason, UnixTimestamp,
};

use super::artifact_maintenance::{active_artifacts_are_usable, collect_obsolete_artifacts};
use super::{SearchArtifactRecoverySummary, RECOVERY_PUBLICATION_LIMIT};
use crate::search_artifacts::write_rebuilt_search_artifacts_from_base;
use crate::search_publication::{commit_prepared_search_publication, load_search_publication_base};
use crate::{ImportPipelineError, Result, SearchPublicationVectorization};

pub fn reconcile_search_artifacts(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactRecoverySummary> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::RepairBlocked {
        return Ok(SearchArtifactRecoverySummary::default());
    }
    let publication_session = match store.try_acquire_search_publication_session() {
        Ok(session) => session,
        Err(error)
            if error.class() == meta_store::MetaStoreErrorClass::MigrationOwnershipRequired =>
        {
            return Ok(SearchArtifactRecoverySummary::default());
        }
        Err(error) => return Err(ImportPipelineError::store(error)),
    };
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::RepairBlocked {
        return Ok(SearchArtifactRecoverySummary::default());
    }
    let interrupted = store
        .interrupted_search_publications(RECOVERY_PUBLICATION_LIMIT)
        .map_err(ImportPipelineError::store)?;
    for publication in &interrupted {
        publication_session
            .abandon_search_publication(&publication.generation, now)
            .map_err(ImportPipelineError::store)?;
    }
    let mut summary = SearchArtifactRecoverySummary {
        interrupted_publications_abandoned: interrupted.len(),
        ..SearchArtifactRecoverySummary::default()
    };

    if state.generation.is_none() {
        if state.service_state == SearchProjectionServiceState::Ready {
            return Err(ImportPipelineError::store_invariant());
        }
        return Ok(summary);
    }

    let artifact_repair_in_progress = state.service_state
        == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::ArtifactUnavailable);
    if artifact_repair_in_progress
        || !active_artifacts_are_usable(&publication_session, vectorization)?
    {
        let base = load_search_publication_base(store)?;
        let classifier_epoch = base.classifier_epoch.clone();
        let generation = base
            .generation
            .as_deref()
            .ok_or_else(ImportPipelineError::store_invariant)?;
        if store
            .begin_artifact_repair(generation, base.visible_epoch, now)
            .map_err(ImportPipelineError::store)?
            == SearchProjectionTransitionOutcome::Superseded
        {
            return Ok(summary);
        }
        let publication = write_rebuilt_search_artifacts_from_base(
            &publication_session,
            now,
            &classifier_epoch,
            &Default::default(),
            Vec::new(),
            base,
            vectorization,
        )?;
        let committed = commit_prepared_search_publication(now, publication, &[])?;
        summary.active_generation_rebuilt = true;
        collect_obsolete_artifacts(&publication_session, &mut summary)?;
        committed.release();
        return Ok(summary);
    }

    collect_obsolete_artifacts(&publication_session, &mut summary)?;
    Ok(summary)
}

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;
