//! Metadata decisions made against a borrowed search-publication view.

use std::collections::{BTreeMap, BTreeSet};

use meta_store::{
    ContentDigest, Document, DocumentId, MigrationRebuildBarrierToken, OwnedMetaStore,
    ProjectedDocumentSnapshot, SearchPublicationCommit, SearchPublicationOutcome,
    SearchPublicationSession, TerminalDocumentUpdate, UnixTimestamp,
};

use super::search_publication::{
    ProjectedDocumentPlan, SearchPublicationDecision, SearchPublicationView,
};
use super::{ImportPipelineError, Result};

pub(super) fn decide_search_publication(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
) -> Result<SearchPublicationDecision> {
    decide_search_publication_after(publication, now, documents, || Ok(()))
}

pub(super) fn decide_search_publication_cancellable(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
) -> Result<SearchPublicationDecision> {
    decide_search_publication_after(publication, now, documents, ensure_not_cancelled)
}

fn decide_search_publication_after(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    before_commit: impl FnOnce() -> Result<()>,
) -> Result<SearchPublicationDecision> {
    decide_search_publication_with(publication, now, documents, |session, commit| {
        before_commit()?;
        session
            .commit_search_publication(commit)
            .map_err(ImportPipelineError::store)
    })
}

pub(super) fn decide_migration_rebuild_search_publication(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    barrier: &MigrationRebuildBarrierToken,
) -> Result<SearchPublicationDecision> {
    decide_search_publication_with(publication, now, documents, |session, commit| {
        session
            .commit_migration_rebuild_search_publication(commit, barrier)
            .map_err(ImportPipelineError::store)
    })
}

#[cfg(test)]
pub(super) fn decide_migration_rebuild_search_publication_with_outcome_for_test(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    outcome: SearchPublicationOutcome,
) -> Result<SearchPublicationDecision> {
    decide_search_publication_with(publication, now, documents, |_, _| Ok(outcome))
}

#[cfg(test)]
pub(super) fn decide_search_publication_with_for_test(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    commit_publication: impl FnOnce(
        &SearchPublicationSession,
        &SearchPublicationCommit<'_>,
    ) -> Result<SearchPublicationOutcome>,
) -> Result<SearchPublicationDecision> {
    decide_search_publication_with(publication, now, documents, commit_publication)
}

fn decide_search_publication_with(
    publication: &SearchPublicationView<'_>,
    now: UnixTimestamp,
    documents: &[Document],
    commit_publication: impl FnOnce(
        &SearchPublicationSession,
        &SearchPublicationCommit<'_>,
    ) -> Result<SearchPublicationOutcome>,
) -> Result<SearchPublicationDecision> {
    let publication_session = publication.publication_session();
    let fulltext = publication.fulltext();
    let vector = publication.vector();
    let projections = publication.projections();
    if fulltext.document_count() != projections.len()
        || vector.projection_count() != projections.len()
        || fulltext.generation() != vector.generation()
        || fulltext.generation().is_empty()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let publication_documents = publication_document_map(documents)?;
    let projected_documents = bind_projected_document_snapshots(
        publication.projected_documents(),
        &publication_documents,
    )?;
    let metadata_changed_ids = projected_documents
        .iter()
        .filter_map(|snapshot| match snapshot {
            ProjectedDocumentSnapshot::MetadataChanged { projection, .. } => {
                Some(&projection.document_id)
            }
            ProjectedDocumentSnapshot::RetainedUnchanged { .. }
            | ProjectedDocumentSnapshot::Replacement { .. } => None,
        })
        .collect::<BTreeSet<_>>();
    let terminal_targets = documents
        .iter()
        .filter(|document| !metadata_changed_ids.contains(&document.id))
        .cloned()
        .collect::<Vec<_>>();
    let terminal_documents =
        terminal_document_updates(publication_session.owned_store(), &terminal_targets)?;
    let commit = SearchPublicationCommit {
        generation: fulltext.generation(),
        terminal_documents: &terminal_documents,
        projections,
        projected_documents: &projected_documents,
        vector_coverage: publication.vector_coverage(),
        now,
    };
    Ok(match commit_publication(publication_session, &commit)? {
        SearchPublicationOutcome::Applied => SearchPublicationDecision::Applied,
        SearchPublicationOutcome::Superseded => SearchPublicationDecision::NotApplied,
    })
}

pub(super) fn publication_document_map(
    documents: &[Document],
) -> Result<BTreeMap<&DocumentId, &Document>> {
    let mut mapped = BTreeMap::new();
    for document in documents {
        if mapped.insert(&document.id, document).is_some() {
            return Err(ImportPipelineError::store_invariant());
        }
    }
    Ok(mapped)
}

pub(super) fn bind_projected_document_snapshots(
    plans: &[ProjectedDocumentPlan],
    documents: &BTreeMap<&DocumentId, &Document>,
) -> Result<Vec<ProjectedDocumentSnapshot>> {
    plans
        .iter()
        .map(|plan| {
            let projection = plan.projection().clone();
            match plan {
                ProjectedDocumentPlan::RetainedUnchanged(_) => {
                    if documents.contains_key(&projection.document_id) {
                        return Err(ImportPipelineError::store_invariant());
                    }
                    Ok(ProjectedDocumentSnapshot::RetainedUnchanged { projection })
                }
                ProjectedDocumentPlan::MetadataChanged(_) => {
                    let document = documents
                        .get(&projection.document_id)
                        .copied()
                        .ok_or_else(ImportPipelineError::store_invariant)?
                        .clone();
                    Ok(ProjectedDocumentSnapshot::MetadataChanged {
                        projection,
                        document,
                    })
                }
                ProjectedDocumentPlan::Replacement(_) => {
                    let document = documents
                        .get(&projection.document_id)
                        .copied()
                        .ok_or_else(ImportPipelineError::store_invariant)?
                        .clone();
                    Ok(ProjectedDocumentSnapshot::Replacement {
                        projection,
                        document,
                    })
                }
            }
        })
        .collect()
}

fn terminal_document_updates(
    store: &OwnedMetaStore,
    documents: &[Document],
) -> Result<Vec<TerminalDocumentUpdate>> {
    let mut updates = Vec::with_capacity(documents.len());
    let mut seen = BTreeSet::new();
    for target in documents {
        if !seen.insert(target.id.clone()) {
            return Err(ImportPipelineError::store_invariant());
        }
        let current = store
            .document_by_id(&target.id)
            .map_err(ImportPipelineError::store)?
            .ok_or_else(ImportPipelineError::store_invariant)?;
        let expected_content_hash = current
            .content_hash
            .as_deref()
            .ok_or_else(ImportPipelineError::store_invariant)?
            .parse::<ContentDigest>()
            .map_err(|_| ImportPipelineError::store_invariant())?;
        updates.push(TerminalDocumentUpdate {
            document_id: target.id.clone(),
            expected_status: current.status,
            expected_is_deleted: current.is_deleted,
            expected_content_hash,
            terminal_status: target.status,
            terminal_is_deleted: target.is_deleted,
        });
    }
    Ok(updates)
}
