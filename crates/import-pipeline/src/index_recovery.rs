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
use meta_store::{
    MetaStore, SearchProjectionServiceState, SearchPublicationState, SearchRepairReason,
    UnixTimestamp, VectorSnapshotMode,
};

use super::index_publication::SearchPublicationLock;
use super::search_artifacts::write_rebuilt_search_artifacts_from_base;
use super::search_publication::{commit_prepared_search_publication, load_search_publication_base};
use super::{ImportPipelineError, Result, SearchPublicationVectorization};

const RECOVERY_PUBLICATION_LIMIT: usize = 256;
const RETAIN_READY_GENERATIONS: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchArtifactRecoverySummary {
    pub interrupted_publications_abandoned: usize,
    pub fulltext_staging_directories_removed: usize,
    pub vector_staging_directories_removed: usize,
    pub fulltext_generations_removed: usize,
    pub vector_generations_removed: usize,
    pub active_generation_rebuilt: bool,
    pub gc_deferred: bool,
    pub gc_partial: bool,
}

pub fn reconcile_search_artifacts(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactRecoverySummary> {
    let publication_lock =
        SearchPublicationLock::acquire(data_dir).map_err(|_| ImportPipelineError::index_io())?;
    let interrupted = store
        .interrupted_search_publications(RECOVERY_PUBLICATION_LIMIT)
        .map_err(ImportPipelineError::store)?;
    for publication in &interrupted {
        store
            .abandon_search_publication(&publication.generation, now)
            .map_err(ImportPipelineError::store)?;
    }
    let mut summary = SearchArtifactRecoverySummary {
        interrupted_publications_abandoned: interrupted.len(),
        ..SearchArtifactRecoverySummary::default()
    };

    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.generation.is_none() {
        if state.service_state == SearchProjectionServiceState::Ready {
            return Err(ImportPipelineError::store_invariant());
        }
        return Ok(summary);
    }

    if !active_artifacts_are_usable(data_dir, store, vectorization)? {
        let base = load_search_publication_base(store)?;
        let classifier_epoch = base.classifier_epoch.clone();
        store
            .mark_search_repairing(SearchRepairReason::ArtifactUnavailable, now)
            .map_err(ImportPipelineError::store)?;
        let publication = write_rebuilt_search_artifacts_from_base(
            data_dir,
            store,
            now,
            &classifier_epoch,
            publication_lock,
            &Default::default(),
            Vec::new(),
            base,
            vectorization,
        )?;
        let committed = commit_prepared_search_publication(store, now, publication, &[])?;
        summary.active_generation_rebuilt = true;
        collect_obsolete_artifacts(data_dir, store, &mut summary)?;
        committed.release();
        return Ok(summary);
    }

    collect_obsolete_artifacts(data_dir, store, &mut summary)?;
    Ok(summary)
}

fn active_artifacts_are_usable(
    data_dir: &Path,
    store: &MetaStore,
    vectorization: &SearchPublicationVectorization,
) -> Result<bool> {
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

fn collect_obsolete_artifacts(
    data_dir: &Path,
    store: &MetaStore,
    summary: &mut SearchArtifactRecoverySummary,
) -> Result<()> {
    let fulltext_root = data_dir.join("search-index");
    let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index"))
        .map_err(ImportPipelineError::vector)?;
    let Some(fulltext_acquisition) =
        try_acquire_fulltext_gc(&fulltext_root).map_err(ImportPipelineError::index)?
    else {
        summary.gc_deferred = true;
        return Ok(());
    };
    let Some(vector_acquisition) = vector_root
        .try_acquire_snapshot_gc()
        .map_err(ImportPipelineError::vector)?
    else {
        summary.gc_deferred = true;
        return Ok(());
    };
    let retained = store
        .search_artifact_retention_generations(RETAIN_READY_GENERATIONS)
        .map_err(ImportPipelineError::store)?;
    let fulltext_prepared = match prepare_fulltext_gc(fulltext_acquisition, &retained)
        .map_err(ImportPipelineError::index)?
    {
        FullTextSnapshotGcPreparation::Deferred => {
            summary.gc_deferred = true;
            return Ok(());
        }
        FullTextSnapshotGcPreparation::Prepared(prepared) => prepared,
    };
    let vector_prepared = match vector_root
        .prepare_snapshot_gc(vector_acquisition, &retained)
        .map_err(ImportPipelineError::vector)?
    {
        VectorSnapshotGcPreparation::Deferred => {
            drop(fulltext_prepared);
            summary.gc_deferred = true;
            return Ok(());
        }
        VectorSnapshotGcPreparation::Prepared(prepared) => prepared,
    };

    match commit_fulltext_gc(fulltext_prepared) {
        FullTextSnapshotGcCommitReport::Complete(progress) => {
            summary.fulltext_generations_removed += progress.removed_snapshots();
            summary.fulltext_staging_directories_removed += progress.removed_staging();
        }
        FullTextSnapshotGcCommitReport::PartialFailure(failure) => {
            summary.fulltext_generations_removed += failure.progress().removed_snapshots();
            summary.fulltext_staging_directories_removed += failure.progress().removed_staging();
            summary.gc_partial = true;
        }
    }
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
    Ok(())
}
