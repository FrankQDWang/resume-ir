use std::collections::{BTreeMap, BTreeSet};

use index_fulltext::{IndexDocument, PublishedSnapshotMetadata};
use index_vector::{VectorModelContract, VectorSnapshotSummary};
use meta_store::{
    ActiveSearchProjection, ContentDigest, Document, DocumentId, MigrationRebuildBarrierToken,
    OwnedMetaStore, ProjectedDocumentSnapshot, ResumeVersionId, SearchProjectionDigest,
    SearchProjectionServiceState, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationLease, SearchPublicationOutcome, SearchPublicationSession,
    SearchPublicationState, SearchPublicationValidation, SearchRepairReason,
    TerminalDocumentUpdate, UnixTimestamp,
};

use super::search_publication_vector::{
    meta_vector_descriptor, publish_vector_generation, validate_vector_publication,
    vector_model_contract, StagedSearchVersionTexts,
};
use super::{ImportPipelineError, Result, SearchPublicationVectorization};

pub(super) struct PreparedSearchPublication<'session> {
    pub(super) publication_session: &'session SearchPublicationSession,
    pub(super) _publication_lease: SearchPublicationLease,
    pub(super) fulltext: PublishedSnapshotMetadata,
    pub(super) vector: VectorSnapshotSummary,
    pub(super) projections: Vec<ActiveSearchProjection>,
    pub(super) projected_documents: Vec<ProjectedDocumentPlan>,
    pub(super) vector_coverage: Vec<ActiveSearchProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProjectedDocumentPlan {
    RetainedUnchanged(ActiveSearchProjection),
    MetadataChanged(ActiveSearchProjection),
    Replacement(ActiveSearchProjection),
}

impl ProjectedDocumentPlan {
    fn projection(&self) -> &ActiveSearchProjection {
        match self {
            Self::RetainedUnchanged(projection)
            | Self::MetadataChanged(projection)
            | Self::Replacement(projection) => projection,
        }
    }
}

#[must_use = "release the committed publication fence after dependent work is complete"]
pub(super) struct CommittedSearchPublication {
    pub(super) _publication_lease: SearchPublicationLease,
    pub(super) fulltext: PublishedSnapshotMetadata,
    pub(super) projections: Vec<ActiveSearchProjection>,
}

pub(super) struct PublishedSearchPublication {
    pub(super) fulltext: PublishedSnapshotMetadata,
    pub(super) projections: Vec<ActiveSearchProjection>,
}

impl CommittedSearchPublication {
    pub(super) fn release(self) -> PublishedSearchPublication {
        let Self {
            _publication_lease,
            fulltext,
            projections,
        } = self;
        drop(_publication_lease);
        PublishedSearchPublication {
            fulltext,
            projections,
        }
    }
}

pub(super) struct SearchPublicationBase {
    pub(super) generation: Option<String>,
    pub(super) visible_epoch: u64,
    pub(super) classifier_epoch: String,
    pub(super) projections: Vec<ActiveSearchProjection>,
    pub(super) vector_contract: VectorModelContract,
}

pub(super) fn load_search_publication_base(
    store: &OwnedMetaStore,
) -> Result<SearchPublicationBase> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::ArtifactUnavailable)
    {
        return load_artifact_recovery_publication_base(store, state);
    }
    if state.service_state != SearchProjectionServiceState::Ready
        || state.repair_reason.is_some()
        || state.generation.is_none()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let (generation, visible_epoch, classifier_epoch, projections, vector_contract) = store
        .with_search_metadata_snapshot(|snapshot| {
            let vector = snapshot
                .head()
                .publication
                .vector
                .as_ref()
                .ok_or_else(ImportPipelineError::store_invariant)?;
            Ok::<_, ImportPipelineError>((
                snapshot.head().generation.clone(),
                snapshot.head().visible_epoch,
                snapshot.head().publication.classifier_epoch.clone(),
                snapshot
                    .validated_active_projections()
                    .map_err(ImportPipelineError::store)?,
                vector_model_contract(vector)?,
            ))
        })
        .map_err(|error| match error {
            meta_store::SearchMetadataTransactionError::Store(error) => {
                ImportPipelineError::store(error)
            }
            meta_store::SearchMetadataTransactionError::Operation(error) => error,
            meta_store::SearchMetadataTransactionError::Unavailable(_) => {
                ImportPipelineError::store_invariant()
            }
        })?;
    if state.generation.as_deref() != Some(generation.as_str())
        || state.visible_epoch != visible_epoch
    {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(SearchPublicationBase {
        generation: Some(generation),
        visible_epoch,
        classifier_epoch,
        projections,
        vector_contract,
    })
}

pub(super) fn load_migration_rebuild_publication_base(
    store: &OwnedMetaStore,
) -> Result<SearchPublicationBase> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    migration_rebuild_publication_base_from_state(state)
}

pub(super) fn migration_rebuild_publication_base_from_state(
    state: meta_store::SearchProjectionState,
) -> Result<SearchPublicationBase> {
    if state.service_state != SearchProjectionServiceState::Repairing
        || state.repair_reason != Some(SearchRepairReason::MigrationRebuild)
        || state.generation.is_some()
        || state.publication.is_some()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(SearchPublicationBase {
        generation: None,
        visible_epoch: state.visible_epoch,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        projections: Vec::new(),
        vector_contract: VectorModelContract::Disabled,
    })
}

fn load_artifact_recovery_publication_base(
    store: &OwnedMetaStore,
    state: meta_store::SearchProjectionState,
) -> Result<SearchPublicationBase> {
    let generation = state
        .generation
        .ok_or_else(ImportPipelineError::store_invariant)?;
    let publication = state
        .publication
        .ok_or_else(ImportPipelineError::store_invariant)?;
    let fulltext = publication
        .fulltext
        .as_ref()
        .ok_or_else(ImportPipelineError::store_invariant)?;
    let vector = publication
        .vector
        .as_ref()
        .ok_or_else(ImportPipelineError::store_invariant)?;
    if publication.state != SearchPublicationState::Ready
        || publication.generation != generation
        || publication.expected_visible_epoch.checked_add(1) != Some(state.visible_epoch)
        || fulltext.generation() != generation
        || vector.generation() != generation
    {
        return Err(ImportPipelineError::store_invariant());
    }

    let projections = store
        .searchable_document_ids()
        .map_err(ImportPipelineError::store)?
        .into_iter()
        .map(|document_id| {
            store
                .active_search_projection_for_document(&document_id)
                .map_err(ImportPipelineError::store)?
                .ok_or_else(ImportPipelineError::store_invariant)
        })
        .collect::<Result<Vec<_>>>()?;
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .map_err(|_| ImportPipelineError::store_invariant())?;
    if fulltext.document_count() != projections.len() as u64
        || vector.projection_count() != projections.len() as u64
        || fulltext.projection_digest() != &projection_digest
        || vector.projection_digest() != &projection_digest
        || publication.projection_digest != projection_digest
    {
        return Err(ImportPipelineError::store_invariant());
    }

    Ok(SearchPublicationBase {
        generation: Some(generation),
        visible_epoch: state.visible_epoch,
        classifier_epoch: publication.classifier_epoch.clone(),
        projections,
        vector_contract: vector_model_contract(vector)?,
    })
}

pub(super) fn prepare_search_publication<'session>(
    publication_session: &'session SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    base: SearchPublicationBase,
    generation: &str,
    projections: Vec<ActiveSearchProjection>,
    staged_version_texts: &StagedSearchVersionTexts,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    write_fulltext: impl FnOnce() -> Result<PublishedSnapshotMetadata>,
) -> Result<PreparedSearchPublication<'session>> {
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    validate_classifier_epoch(&base, classifier_epoch)?;
    let projected_documents =
        projected_document_plan(&base.projections, &projections, staged_version_texts)?;
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .map_err(|_| ImportPipelineError::store_invariant())?;
    let draft = SearchPublicationDraft {
        generation: generation.to_string(),
        base_generation: base.generation.clone(),
        expected_visible_epoch: base.visible_epoch,
        classifier_epoch: classifier_epoch.to_string(),
        projection_digest: projection_digest.clone(),
        now,
    };
    if publication_session
        .begin_search_publication(&draft)
        .map_err(ImportPipelineError::store)?
        == SearchPublicationOutcome::Superseded
    {
        return Err(ImportPipelineError::index_io());
    }

    let result = (|| {
        let fulltext = write_fulltext()?;
        if fulltext.generation() != generation
            || fulltext.document_count() != projections.len()
            || fulltext.projection_digest() != &projection_digest
        {
            return Err(ImportPipelineError::store_invariant());
        }
        let vector = publish_vector_generation(
            data_dir,
            store,
            generation,
            &base,
            &projections,
            staged_version_texts,
            vectorization,
            ensure_not_cancelled,
        )?;
        let vector_coverage =
            validate_vector_publication(&vector, generation, &projections, &projection_digest)?;
        let fulltext_descriptor = meta_fulltext_descriptor(&fulltext)?;
        let vector_descriptor = meta_vector_descriptor(&vector)?;
        publication_session
            .validate_search_publication(&SearchPublicationValidation {
                generation,
                fulltext: &fulltext_descriptor,
                vector: &vector_descriptor,
                now,
            })
            .map_err(ImportPipelineError::store)?;
        Ok(PreparedSearchPublication {
            publication_session,
            _publication_lease: publication_session.retain(),
            fulltext,
            vector,
            projections,
            projected_documents,
            vector_coverage,
        })
    })();
    if result.is_err() {
        publication_session
            .abandon_search_publication(generation, now)
            .map_err(ImportPipelineError::store)?;
    }
    result
}

fn projected_document_plan(
    base: &[ActiveSearchProjection],
    target: &[ActiveSearchProjection],
    staged_version_texts: &StagedSearchVersionTexts,
) -> Result<Vec<ProjectedDocumentPlan>> {
    let base = base
        .iter()
        .map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    target
        .iter()
        .map(|projection| {
            let staged = staged_version_texts
                .get(projection.resume_version_id.as_str())
                .is_some_and(|staged| staged.document_id == projection.document_id);
            match (base.get(projection.document_id.as_str()).copied(), staged) {
                (Some(version), false) if version == projection.resume_version_id.as_str() => {
                    Ok(ProjectedDocumentPlan::RetainedUnchanged(projection.clone()))
                }
                (Some(version), true) if version == projection.resume_version_id.as_str() => {
                    Ok(ProjectedDocumentPlan::MetadataChanged(projection.clone()))
                }
                (Some(version), true) if version != projection.resume_version_id.as_str() => {
                    Ok(ProjectedDocumentPlan::Replacement(projection.clone()))
                }
                (None, true) => Ok(ProjectedDocumentPlan::Replacement(projection.clone())),
                _ => Err(ImportPipelineError::store_invariant()),
            }
        })
        .collect()
}

fn validate_classifier_epoch(base: &SearchPublicationBase, classifier_epoch: &str) -> Result<()> {
    if !base.projections.is_empty() && classifier_epoch != base.classifier_epoch {
        Err(ImportPipelineError::store_invariant())
    } else {
        Ok(())
    }
}

pub(super) fn projections_after_delta(
    base: &[ActiveSearchProjection],
    replacements: &[IndexDocument],
    removals: &BTreeSet<String>,
) -> Result<Vec<ActiveSearchProjection>> {
    let replacement_ids = replacements
        .iter()
        .map(|document| document.doc_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut projections = Vec::with_capacity(base.len() + replacements.len());
    for projection in base {
        if removals.contains(projection.document_id.as_str())
            || replacement_ids.contains(projection.document_id.as_str())
        {
            continue;
        }
        projections.push(projection.clone());
    }
    for replacement in replacements {
        let document_id = replacement
            .doc_id
            .parse::<DocumentId>()
            .map_err(|_| ImportPipelineError::store_invariant())?;
        let resume_version_id = replacement
            .resume_version_id
            .parse::<ResumeVersionId>()
            .map_err(|_| ImportPipelineError::store_invariant())?;
        projections.push(ActiveSearchProjection {
            document_id,
            resume_version_id,
        });
    }
    projections.sort_by(|left, right| left.document_id.cmp(&right.document_id));
    if projections
        .windows(2)
        .any(|pair| pair[0].document_id == pair[1].document_id)
    {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(projections)
}

pub(super) fn commit_prepared_search_publication(
    now: UnixTimestamp,
    publication: PreparedSearchPublication<'_>,
    documents: &[Document],
) -> Result<CommittedSearchPublication> {
    commit_prepared_search_publication_with(now, publication, documents, |session, commit| {
        session.commit_search_publication(commit)
    })
    .and_then(|committed| committed.ok_or_else(ImportPipelineError::index_io))
}

pub(super) fn commit_migration_rebuild_search_publication(
    now: UnixTimestamp,
    publication: PreparedSearchPublication<'_>,
    documents: &[Document],
    barrier: &MigrationRebuildBarrierToken,
) -> Result<Option<CommittedSearchPublication>> {
    commit_prepared_search_publication_with(now, publication, documents, |session, commit| {
        session.commit_migration_rebuild_search_publication(commit, barrier)
    })
}

fn commit_prepared_search_publication_with(
    now: UnixTimestamp,
    publication: PreparedSearchPublication<'_>,
    documents: &[Document],
    commit_publication: impl FnOnce(
        &SearchPublicationSession,
        &SearchPublicationCommit<'_>,
    ) -> meta_store::Result<SearchPublicationOutcome>,
) -> Result<Option<CommittedSearchPublication>> {
    let PreparedSearchPublication {
        publication_session,
        _publication_lease,
        fulltext,
        vector,
        projections,
        projected_documents,
        vector_coverage,
    } = publication;
    let store = publication_session.owned_store();
    if fulltext.document_count() != projections.len()
        || vector.projection_count() != projections.len()
        || fulltext.generation() != vector.generation()
        || fulltext.generation().is_empty()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let publication_documents = publication_document_map(documents)?;
    let projected_documents =
        bind_projected_document_snapshots(&projected_documents, &publication_documents)?;
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
    let terminal_documents = terminal_document_updates(store, &terminal_targets)?;
    let commit = SearchPublicationCommit {
        generation: fulltext.generation(),
        terminal_documents: &terminal_documents,
        projections: &projections,
        projected_documents: &projected_documents,
        vector_coverage: &vector_coverage,
        now,
    };
    match commit_publication(publication_session, &commit).map_err(ImportPipelineError::store)? {
        SearchPublicationOutcome::Applied => Ok(Some(CommittedSearchPublication {
            _publication_lease,
            fulltext,
            projections,
        })),
        SearchPublicationOutcome::Superseded => Ok(None),
    }
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

fn meta_fulltext_descriptor(
    metadata: &PublishedSnapshotMetadata,
) -> Result<meta_store::FullTextSnapshotDescriptor> {
    let document_count = u64::try_from(metadata.document_count())
        .map_err(|_| ImportPipelineError::store_invariant())?;
    Ok(meta_store::FullTextSnapshotDescriptor::new(
        metadata.generation().to_string(),
        document_count,
        metadata.projection_digest().clone(),
        metadata.logical_content_digest().clone(),
    ))
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

#[cfg(test)]
mod tests {
    use meta_store::{
        ActiveSearchProjection, ContentDigest, DocumentId, ResumeVersionId, SourceRevisionId,
    };

    use super::{validate_classifier_epoch, SearchPublicationBase};

    fn base_with_projection() -> SearchPublicationBase {
        let document_id = DocumentId::from_non_secret_parts(&["document-a"]);
        let source_digest = ContentDigest::from_bytes(b"source-a");
        let source_revision_id =
            SourceRevisionId::from_content_identity(&document_id, &source_digest);
        let normalized_text = ContentDigest::from_bytes(b"normalized-a");
        SearchPublicationBase {
            generation: Some("generation-a".to_string()),
            visible_epoch: 1,
            classifier_epoch: "classifier-a".to_string(),
            projections: vec![ActiveSearchProjection {
                resume_version_id: ResumeVersionId::from_content_identity(
                    &document_id,
                    &source_revision_id,
                    &normalized_text,
                    "parser-a",
                    "schema-a",
                ),
                document_id,
            }],
            vector_contract: index_vector::VectorModelContract::Disabled,
        }
    }

    #[test]
    fn retained_projection_rejects_classifier_epoch_change() {
        assert!(validate_classifier_epoch(&base_with_projection(), "classifier-b").is_err());
        assert!(validate_classifier_epoch(&base_with_projection(), "classifier-a").is_ok());
    }

    #[test]
    fn empty_projection_allows_classifier_epoch_change() {
        let mut base = base_with_projection();
        base.projections.clear();

        assert!(validate_classifier_epoch(&base, "classifier-b").is_ok());
    }
}
