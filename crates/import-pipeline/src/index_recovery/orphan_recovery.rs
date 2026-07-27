//! Strict recovery of complete current-format artifact pairs left outside the
//! metadata publication journal.

use index_fulltext::{FullTextIndex, PublishedSnapshotMetadata, SnapshotReadLease};
use index_vector::{VectorSnapshotReader, VectorSnapshotRoot, VectorSnapshotSummary};
use meta_store::{
    SearchPublicationRecord, SearchPublicationSession, SearchPublicationState, UnixTimestamp,
};

use super::RECOVERY_PUBLICATION_LIMIT;
use crate::search_publication::{
    run_recovered_search_publication_transaction, CommittedSearchPublication, SearchPublicationBase,
};
use crate::search_publication_commit::decide_search_publication;
use crate::search_publication_vector::vector_model_contract;
use crate::{ImportPipelineError, PipelineRunControl, Result, SearchPublicationVectorization};

pub(super) fn recover_matching_orphan_artifact_pair(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    expected: &SearchPublicationRecord,
    base: &SearchPublicationBase,
    vectorization: &SearchPublicationVectorization,
    control: &PipelineRunControl,
) -> Result<Option<CommittedSearchPublication>> {
    if expected.state != SearchPublicationState::Ready {
        return Ok(None);
    }
    let (Some(expected_fulltext), Some(expected_vector)) =
        (expected.fulltext.as_ref(), expected.vector.as_ref())
    else {
        return Ok(None);
    };
    let data_dir = publication_session.canonical_data_dir();
    let fulltext_root = data_dir.join("search-index");
    let Some(inspection_lease) =
        SnapshotReadLease::acquire(&fulltext_root).map_err(ImportPipelineError::index)?
    else {
        return Ok(None);
    };
    let candidates = match FullTextIndex::inspect_snapshot_manifests_with_lease(
        &fulltext_root,
        &inspection_lease,
        RECOVERY_PUBLICATION_LIMIT,
    ) {
        Ok(candidates) => candidates,
        Err(_) => return Ok(None),
    };
    drop(inspection_lease);
    let vector_contract = vector_model_contract(expected_vector)?;
    if let Some(vectorizer) = vectorization.vectorizer() {
        let configured = index_vector::VectorModelContract::enabled(
            vectorizer.model_id(),
            vectorizer.dimension(),
        )
        .map_err(ImportPipelineError::vector)?;
        if configured != vector_contract {
            return Ok(None);
        }
    }
    let vector_root = match VectorSnapshotRoot::new(data_dir.join("vector-index")) {
        Ok(root) => root,
        Err(_) => return Ok(None),
    };

    for candidate in candidates {
        control.ensure_running()?;
        if candidate.generation() == expected.generation
            || !fulltext_candidate_matches(&candidate, expected_fulltext)
            || publication_session
                .owned_store()
                .search_publication(candidate.generation())
                .map_err(ImportPipelineError::store)?
                .is_some()
        {
            continue;
        }
        let Some(fulltext_lease) =
            SnapshotReadLease::acquire(&fulltext_root).map_err(ImportPipelineError::index)?
        else {
            return Ok(None);
        };
        let fulltext_reader = match FullTextIndex::open_snapshot_with_lease(
            &fulltext_root,
            candidate.generation(),
            fulltext_lease,
        ) {
            Ok(Some(reader)) => reader,
            Ok(None) | Err(_) => continue,
        };
        let vector_lease = match vector_root.acquire_read_lease() {
            Ok(lease) => lease,
            Err(_) => continue,
        };
        let vector_reader = match vector_root.open_generation_with_lease(
            candidate.generation(),
            &vector_contract,
            vector_lease,
        ) {
            Ok(reader) => reader,
            Err(_) => continue,
        };
        if !vector_candidate_matches(&vector_reader, expected_vector) {
            continue;
        }
        control.ensure_running()?;
        let committed = run_recovered_search_publication_transaction(
            publication_session,
            now,
            base.clone(),
            fulltext_reader,
            vector_reader,
            |publication| decide_search_publication(publication, now, &[]),
        )?
        .into_committed()?;
        return Ok(Some(committed));
    }
    Ok(None)
}

fn fulltext_candidate_matches(
    candidate: &PublishedSnapshotMetadata,
    expected: &meta_store::FullTextSnapshotDescriptor,
) -> bool {
    u64::try_from(candidate.document_count()).ok() == Some(expected.document_count())
        && candidate.projection_digest() == expected.projection_digest()
        && candidate.logical_content_digest() == expected.logical_content_digest()
}

fn vector_candidate_matches(
    candidate: &VectorSnapshotReader,
    expected: &meta_store::VectorSnapshotDescriptor,
) -> bool {
    let summary: &VectorSnapshotSummary = candidate.summary();
    let Ok(expected_contract) = vector_model_contract(expected) else {
        return false;
    };
    summary.model_contract() == &expected_contract
        && u64::try_from(summary.projection_count()).ok() == Some(expected.projection_count())
        && u64::try_from(summary.vector_count()).ok() == Some(expected.vector_count())
        && u64::try_from(summary.vector_document_count()).ok() == Some(expected.document_count())
        && u64::try_from(summary.vector_document_count()).ok()
            == Some(expected.resume_version_count())
        && summary.projection_digest() == expected.projection_digest()
        && summary.coverage_digest() == expected.coverage_digest()
        && summary.logical_content_digest() == expected.logical_content_digest()
}
