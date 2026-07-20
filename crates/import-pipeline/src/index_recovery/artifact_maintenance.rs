//! Exact active-artifact validation and coordinated full-text/vector garbage collection.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use index_fulltext::{
    commit_snapshot_gc as commit_fulltext_gc,
    commit_snapshot_gc_with_cancel_check as commit_fulltext_gc_with_cancel_check,
    prepare_snapshot_gc as prepare_fulltext_gc, try_acquire_snapshot_gc as try_acquire_fulltext_gc,
    FullTextIndex, FullTextSnapshotGcCommitReport, FullTextSnapshotGcPreparation,
    SnapshotReadLease,
};
use index_vector::{
    commit_snapshot_gc as commit_vector_gc,
    commit_snapshot_gc_with_cancel_check as commit_vector_gc_with_cancel_check,
    VectorModelContract, VectorSnapshotGcCommitReport, VectorSnapshotGcPreparation,
    VectorSnapshotRoot,
};
use meta_store::{SearchPublicationSession, SearchPublicationState, VectorSnapshotMode};

use super::SearchArtifactRecoverySummary;
use crate::{ImportPipelineError, PipelineRunControl, Result, SearchPublicationVectorization};

const RETAIN_READY_GENERATIONS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActiveArtifactValidationDepth {
    ManifestOnly,
    OpenPayloads,
}

pub(super) fn active_artifacts_are_usable(
    publication_session: &SearchPublicationSession,
    vectorization: &SearchPublicationVectorization,
    validation_depth: ActiveArtifactValidationDepth,
) -> Result<bool> {
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    let Some(publication) = state.publication.as_deref() else {
        return Ok(false);
    };
    if publication.state != SearchPublicationState::Ready {
        return Ok(false);
    }
    let (Some(fulltext), Some(vector)) = (&publication.fulltext, &publication.vector) else {
        return Ok(false);
    };

    let fulltext_root = data_dir.join("search-index");
    let Some(fulltext_lease) = SnapshotReadLease::acquire(&fulltext_root).ok().flatten() else {
        return Ok(false);
    };
    let Some(fulltext_manifest) = FullTextIndex::inspect_snapshot_manifest_with_lease(
        &fulltext_root,
        fulltext.generation(),
        &fulltext_lease,
    )
    .ok()
    .flatten() else {
        return Ok(false);
    };
    if fulltext_manifest.generation() != fulltext.generation()
        || u64::try_from(fulltext_manifest.document_count()).ok() != Some(fulltext.document_count())
        || fulltext_manifest.projection_digest() != fulltext.projection_digest()
        || fulltext_manifest.logical_content_digest() != fulltext.logical_content_digest()
    {
        return Ok(false);
    }
    if validation_depth == ActiveArtifactValidationDepth::OpenPayloads
        && FullTextIndex::open_snapshot_with_lease(
            &fulltext_root,
            fulltext.generation(),
            fulltext_lease,
        )
        .ok()
        .flatten()
        .is_none()
    {
        return Ok(false);
    }

    let contract = match vector.mode() {
        VectorSnapshotMode::Disabled => VectorModelContract::Disabled,
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => VectorModelContract::enabled(model_id.clone(), *dimension as usize)
            .map_err(ImportPipelineError::vector)?,
    };
    if let Some(vectorizer) = vectorization.vectorizer() {
        let configured_contract =
            VectorModelContract::enabled(vectorizer.model_id().to_string(), vectorizer.dimension())
                .map_err(ImportPipelineError::vector)?;
        if contract != configured_contract {
            return Ok(false);
        }
    }
    let vector_root = match VectorSnapshotRoot::new(data_dir.join("vector-index")) {
        Ok(root) => root,
        Err(_) => return Ok(false),
    };
    let vector_lease = match vector_root.acquire_read_lease() {
        Ok(lease) => lease,
        Err(_) => return Ok(false),
    };
    let manifest = match vector_root.inspect_generation_manifest_with_lease(
        vector.generation(),
        &contract,
        &vector_lease,
    ) {
        Ok(Some(manifest)) => manifest,
        Err(_) => return Ok(false),
        Ok(None) => return Ok(false),
    };
    let manifest_matches = manifest.generation() == vector.generation()
        && manifest.model_contract() == &contract
        && u64::try_from(manifest.projection_count()).ok() == Some(vector.projection_count())
        && u64::try_from(manifest.vector_count()).ok() == Some(vector.vector_count())
        && u64::try_from(manifest.vector_document_count()).ok() == Some(vector.document_count())
        && manifest.projection_digest() == vector.projection_digest()
        && manifest.coverage_digest() == vector.coverage_digest()
        && manifest.logical_content_digest() == vector.logical_content_digest();
    if !manifest_matches {
        return Ok(false);
    }
    if validation_depth == ActiveArtifactValidationDepth::OpenPayloads
        && vector_root
            .open_generation_with_lease(vector.generation(), &contract, vector_lease)
            .is_err()
    {
        return Ok(false);
    }
    Ok(true)
}

pub(super) fn collect_obsolete_artifacts(
    publication_session: &SearchPublicationSession,
    summary: &mut SearchArtifactRecoverySummary,
    control: Option<&PipelineRunControl>,
) -> Result<()> {
    if let Some(control) = control {
        control.ensure_running()?;
    }
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    let fulltext_root = data_dir.join("search-index");
    let vector_path = data_dir.join("vector-index");
    let fulltext_exists = artifact_root_has_entries(&fulltext_root)?;
    let vector_root = artifact_root_has_entries(&vector_path)?
        .then(|| VectorSnapshotRoot::new(&vector_path).map_err(ImportPipelineError::vector))
        .transpose()?;
    if !fulltext_exists && vector_root.is_none() {
        return Ok(());
    }
    let fulltext_acquisition = if fulltext_exists {
        let Some(acquisition) =
            try_acquire_fulltext_gc(&fulltext_root).map_err(ImportPipelineError::index)?
        else {
            summary.gc_deferred = true;
            return Ok(());
        };
        Some(acquisition)
    } else {
        None
    };
    let vector_acquisition = if let Some(vector_root) = vector_root.as_ref() {
        let Some(acquisition) = vector_root
            .try_acquire_snapshot_gc()
            .map_err(ImportPipelineError::vector)?
        else {
            summary.gc_deferred = true;
            return Ok(());
        };
        Some(acquisition)
    } else {
        None
    };
    let retained = store
        .search_artifact_retention_generations(RETAIN_READY_GENERATIONS)
        .map_err(ImportPipelineError::store)?;
    let fulltext_prepared = match fulltext_acquisition {
        Some(acquisition) => {
            match prepare_fulltext_gc(acquisition, &retained).map_err(ImportPipelineError::index)? {
                FullTextSnapshotGcPreparation::Deferred => {
                    summary.gc_deferred = true;
                    return Ok(());
                }
                FullTextSnapshotGcPreparation::Prepared(prepared) => Some(prepared),
            }
        }
        None => None,
    };
    let vector_prepared = match (vector_root.as_ref(), vector_acquisition) {
        (Some(vector_root), Some(acquisition)) => match vector_root
            .prepare_snapshot_gc(acquisition, &retained)
            .map_err(ImportPipelineError::vector)?
        {
            VectorSnapshotGcPreparation::Deferred => {
                drop(fulltext_prepared);
                summary.gc_deferred = true;
                return Ok(());
            }
            VectorSnapshotGcPreparation::Prepared(prepared) => Some(prepared),
        },
        (None, None) => None,
        _ => return Err(ImportPipelineError::store_invariant()),
    };

    if let Some(fulltext_prepared) = fulltext_prepared {
        let report = match control {
            Some(control) => commit_fulltext_gc_with_cancel_check(fulltext_prepared, &|| {
                control.shutdown_requested()
            }),
            None => commit_fulltext_gc(fulltext_prepared),
        };
        match report {
            FullTextSnapshotGcCommitReport::Complete(progress) => {
                summary.fulltext_generations_removed += progress.removed_snapshots();
                summary.fulltext_staging_directories_removed += progress.removed_staging();
            }
            FullTextSnapshotGcCommitReport::Interrupted(progress) => {
                summary.fulltext_generations_removed += progress.removed_snapshots();
                summary.fulltext_staging_directories_removed += progress.removed_staging();
                return Err(ImportPipelineError::interrupted());
            }
            FullTextSnapshotGcCommitReport::PartialFailure(failure) => {
                summary.fulltext_generations_removed += failure.progress().removed_snapshots();
                summary.fulltext_staging_directories_removed +=
                    failure.progress().removed_staging();
                summary.gc_partial = true;
            }
        }
    }
    if let Some(vector_prepared) = vector_prepared {
        let report = match control {
            Some(control) => commit_vector_gc_with_cancel_check(vector_prepared, &|| {
                control.shutdown_requested()
            }),
            None => commit_vector_gc(vector_prepared),
        };
        match report {
            VectorSnapshotGcCommitReport::Complete(progress) => {
                summary.vector_generations_removed += progress.removed_generations();
                summary.vector_staging_directories_removed += progress.removed_staging();
            }
            VectorSnapshotGcCommitReport::Interrupted(progress) => {
                summary.vector_generations_removed += progress.removed_generations();
                summary.vector_staging_directories_removed += progress.removed_staging();
                return Err(ImportPipelineError::interrupted());
            }
            VectorSnapshotGcCommitReport::PartialFailure(failure) => {
                summary.vector_generations_removed += failure.progress().removed_generations();
                summary.vector_staging_directories_removed += failure.progress().removed_staging();
                summary.gc_partial = true;
            }
        }
    }
    Ok(())
}

/// Keeps reproducible-artifact retirement outside the query availability
/// boundary. Cancellation still unwinds the worker promptly, while any GC
/// storage/layout failure is recorded for a later tick without invalidating an
/// already-validated active generation.
pub(super) fn collect_obsolete_artifacts_best_effort(
    publication_session: &SearchPublicationSession,
    summary: &mut SearchArtifactRecoverySummary,
    control: Option<&PipelineRunControl>,
) -> Result<()> {
    let result = collect_obsolete_artifacts(publication_session, summary, control);
    settle_best_effort_gc(summary, result)
}

fn settle_best_effort_gc(
    summary: &mut SearchArtifactRecoverySummary,
    result: Result<()>,
) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.class(),
                crate::ImportPipelineErrorClass::Cancelled
                    | crate::ImportPipelineErrorClass::Interrupted
            ) =>
        {
            Err(error)
        }
        Err(_) => {
            summary.gc_deferred = true;
            summary.gc_failed = true;
            Ok(())
        }
    }
}

fn artifact_root_has_entries(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            let mut entries = fs::read_dir(path).map_err(|_| ImportPipelineError::index_io())?;
            Ok(entries
                .next()
                .transpose()
                .map_err(|_| ImportPipelineError::index_io())?
                .is_some())
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ImportPipelineError::index_io()),
    }
}

#[cfg(test)]
mod tests {
    use super::settle_best_effort_gc;
    use crate::{ImportPipelineError, ImportPipelineErrorClass};

    #[test]
    fn ready_generation_survives_gc_storage_failure_but_not_lifecycle_cancellation() {
        let mut summary = Default::default();
        settle_best_effort_gc(&mut summary, Err(ImportPipelineError::index_io())).unwrap();
        assert!(summary.gc_deferred);
        assert!(summary.gc_failed);

        let error = settle_best_effort_gc(
            &mut Default::default(),
            Err(ImportPipelineError::interrupted()),
        )
        .unwrap_err();
        assert_eq!(error.class(), ImportPipelineErrorClass::Interrupted);
    }
}
