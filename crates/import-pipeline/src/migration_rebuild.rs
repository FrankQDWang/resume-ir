use meta_store::{
    OwnedMetaStore, SearchProjectionServiceState, SearchProjectionState, SearchRepairReason,
};

use super::{ImportPipelineError, Result};

/// Typed decision returned before an OCR worker mutates its durable queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrPreclaimDecision {
    /// The exact published search head is available for later CAS publication.
    Ready,
    /// The queue must remain untouched until the reported lifecycle clears.
    NotReady(OcrPreclaimNotReady),
}

/// Closed reasons that make OCR queue mutation unsafe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrPreclaimNotReady {
    /// The first or a later search publication is being repaired.
    Repairing,
    /// Repair is sticky-blocked pending source or integrity remediation.
    RepairBlocked,
    /// The service label is ready without a complete generation/publication pair.
    IncompletePublication,
}

/// Decides whether an OCR worker may claim its next durable job.
///
/// Callers must evaluate this boundary before stale-job recovery or claim
/// mutation. A successful claim still revalidates publication readiness while
/// committing its derived version, because the projection may change while OCR
/// is running.
pub fn ocr_preclaim_decision(store: &OwnedMetaStore) -> Result<OcrPreclaimDecision> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    Ok(evaluate_ocr_preclaim_state(&state))
}

pub(super) fn ensure_migration_rebuild_scan_is_complete(
    store: &OwnedMetaStore,
    has_scan_errors: bool,
    scan_budget_exhausted: bool,
) -> Result<()> {
    if !has_scan_errors && !scan_budget_exhausted {
        return Ok(());
    }
    reject_unpublished_migration_source_unavailable(store)
}

fn reject_unpublished_migration_source_unavailable(store: &OwnedMetaStore) -> Result<()> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if !is_unpublished_migration_rebuild(&state) {
        return Ok(());
    }
    Err(ImportPipelineError::migration_scan_incomplete())
}

/// Serializes OCR publication with the first migration publication.
///
/// OCR work may be retried after the exact sealed root projection becomes
/// Ready, but it must never stage a global latest-version side channel while
/// that projection is still unpublished.
pub(super) fn ensure_ocr_publication_ready(store: &OwnedMetaStore) -> Result<()> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if evaluate_ocr_preclaim_state(&state) == OcrPreclaimDecision::Ready {
        return Ok(());
    }
    if !is_unpublished_migration_rebuild(&state) {
        return Err(ImportPipelineError::store_invariant());
    }

    let publication_session = store
        .try_acquire_search_publication_session()
        .map_err(ImportPipelineError::store)?;
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if is_unpublished_migration_rebuild(&state) {
        return Err(ImportPipelineError::repairing());
    }
    drop(publication_session);
    if evaluate_ocr_preclaim_state(&state) == OcrPreclaimDecision::Ready {
        Ok(())
    } else {
        Err(ImportPipelineError::store_invariant())
    }
}

pub(super) fn is_unpublished_migration_rebuild(state: &SearchProjectionState) -> bool {
    state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::MigrationRebuild)
        && state.generation.is_none()
        && state.publication.is_none()
}

fn evaluate_ocr_preclaim_state(state: &SearchProjectionState) -> OcrPreclaimDecision {
    match state.service_state {
        SearchProjectionServiceState::Ready
            if state.repair_reason.is_none()
                && state.generation.is_some()
                && state.publication.is_some() =>
        {
            OcrPreclaimDecision::Ready
        }
        SearchProjectionServiceState::Repairing => {
            OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::Repairing)
        }
        SearchProjectionServiceState::RepairBlocked => {
            OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::RepairBlocked)
        }
        SearchProjectionServiceState::Ready => {
            OcrPreclaimDecision::NotReady(OcrPreclaimNotReady::IncompletePublication)
        }
    }
}

#[cfg(test)]
#[path = "migration_rebuild_tests.rs"]
mod tests;
