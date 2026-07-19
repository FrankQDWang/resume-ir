//! Exact active-artifact validation and coordinated full-text/vector garbage collection.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use index_fulltext::{
    commit_snapshot_gc as commit_fulltext_gc, prepare_snapshot_gc as prepare_fulltext_gc,
    try_acquire_snapshot_gc as try_acquire_fulltext_gc, FullTextIndex,
    FullTextSnapshotGcCommitReport, FullTextSnapshotGcPreparation, SnapshotReadLease,
};
use index_vector::{
    commit_snapshot_gc as commit_vector_gc, VectorModelContract, VectorSnapshotGcCommitReport,
    VectorSnapshotGcPreparation, VectorSnapshotRoot,
};
use meta_store::{SearchPublicationSession, SearchPublicationState, VectorSnapshotMode};

use super::SearchArtifactRecoverySummary;
use crate::{ImportPipelineError, Result, SearchPublicationVectorization};

const RETAIN_READY_GENERATIONS: usize = 2;

pub(super) fn active_artifacts_are_usable(
    publication_session: &SearchPublicationSession,
    vectorization: &SearchPublicationVectorization,
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
    let fulltext_matches = SnapshotReadLease::acquire(&fulltext_root)
        .ok()
        .flatten()
        .and_then(|lease| {
            FullTextIndex::open_snapshot_with_lease(&fulltext_root, fulltext.generation(), lease)
                .ok()
                .flatten()
        })
        .and_then(|index| index.snapshot_metadata().cloned())
        .is_some_and(|metadata| {
            metadata.generation() == fulltext.generation()
                && u64::try_from(metadata.document_count()).ok() == Some(fulltext.document_count())
                && metadata.projection_digest() == fulltext.projection_digest()
                && metadata.logical_content_digest() == fulltext.logical_content_digest()
        });
    if !fulltext_matches {
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
    let reader = match vector_root.acquire_read_lease().and_then(|lease| {
        vector_root.open_generation_with_lease(vector.generation(), &contract, lease)
    }) {
        Ok(reader) => reader,
        Err(_) => return Ok(false),
    };
    let actual = reader.summary();
    Ok(actual.generation() == vector.generation()
        && actual.model_contract() == &contract
        && u64::try_from(actual.projection_count()).ok() == Some(vector.projection_count())
        && u64::try_from(actual.vector_count()).ok() == Some(vector.vector_count())
        && u64::try_from(actual.vector_document_count()).ok() == Some(vector.document_count())
        && actual.projection_digest() == vector.projection_digest()
        && actual.coverage_digest() == vector.coverage_digest()
        && actual.logical_content_digest() == vector.logical_content_digest())
}

pub(super) fn collect_obsolete_artifacts(
    publication_session: &SearchPublicationSession,
    summary: &mut SearchArtifactRecoverySummary,
) -> Result<()> {
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
        match commit_fulltext_gc(fulltext_prepared) {
            FullTextSnapshotGcCommitReport::Complete(progress) => {
                summary.fulltext_generations_removed += progress.removed_snapshots();
                summary.fulltext_staging_directories_removed += progress.removed_staging();
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
        match commit_vector_gc(vector_prepared) {
            VectorSnapshotGcCommitReport::Complete(progress) => {
                summary.vector_generations_removed += progress.removed_generations();
                summary.vector_staging_directories_removed += progress.removed_staging();
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
