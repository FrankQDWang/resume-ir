//! Exact query-to-maintenance handoff for immutable artifact failures.

use core_domain::ContentDigest;
use meta_store::{
    OwnedMetaStore, SearchProjectionServiceState, SearchProjectionTransitionOutcome,
    SearchRepairReason, UnixTimestamp,
};

use crate::{ImportPipelineError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportedArtifactRepairOutcome {
    Started,
    AlreadyRepairing,
    Superseded,
}

/// Moves only the exact still-current ready publication into artifact repair.
///
/// Query threads report immutable identity only; this maintenance-side CAS is
/// the sole metadata writer and never turns a stale fault into repair work for
/// a newer publication.
pub fn begin_reported_artifact_repair(
    store: &OwnedMetaStore,
    generation: &str,
    publication_fingerprint: &ContentDigest,
    now: UnixTimestamp,
) -> Result<ReportedArtifactRepairOutcome> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::ArtifactUnavailable)
    {
        let exact_context = store
            .artifact_repair_context()
            .map_err(ImportPipelineError::store)?
            .is_some_and(|context| {
                context.generation == generation
                    && context.publication_fingerprint == *publication_fingerprint
                    && context.visible_epoch == state.visible_epoch
            });
        return Ok(if exact_context {
            ReportedArtifactRepairOutcome::AlreadyRepairing
        } else {
            ReportedArtifactRepairOutcome::Superseded
        });
    }
    if state.service_state != SearchProjectionServiceState::Ready || state.repair_reason.is_some() {
        return Ok(ReportedArtifactRepairOutcome::Superseded);
    }
    let exact_publication = state.generation.as_deref() == Some(generation)
        && state
            .publication
            .as_ref()
            .and_then(|publication| publication.publication_fingerprint.as_ref())
            == Some(publication_fingerprint);
    if !exact_publication {
        return Ok(ReportedArtifactRepairOutcome::Superseded);
    }
    match store
        .begin_artifact_repair(generation, state.visible_epoch, now)
        .map_err(ImportPipelineError::store)?
    {
        SearchProjectionTransitionOutcome::Applied => Ok(ReportedArtifactRepairOutcome::Started),
        SearchProjectionTransitionOutcome::Superseded => {
            Ok(ReportedArtifactRepairOutcome::Superseded)
        }
    }
}
