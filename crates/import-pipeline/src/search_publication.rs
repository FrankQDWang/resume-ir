use std::collections::BTreeSet;

use index_fulltext::{IndexDocument, PublishedSnapshotMetadata};
use index_vector::{VectorModelContract, VectorSnapshotSummary};
use meta_store::{
    ActiveSearchProjection, ArtifactRepairVectorContext, DocumentId, OwnedMetaStore,
    ResumeVersionId, SearchProjectionDigest, SearchProjectionServiceState, SearchPublicationDraft,
    SearchPublicationLease, SearchPublicationOutcome, SearchPublicationSession,
    SearchPublicationValidation, SearchRepairReason, UnixTimestamp,
};

use super::search_publication_failure::{
    abandon_and_retire_search_publication, FailedGenerationArtifacts,
};
use super::search_publication_vector::{
    meta_vector_descriptor, publish_vector_generation, validate_vector_publication,
    vector_model_contract, StagedSearchVersionTexts,
};
use super::{ImportPipelineError, Result, SearchPublicationVectorization};

#[must_use = "a prepared search publication must be explicitly terminated"]
struct PreparedSearchPublication<'session> {
    publication_session: &'session SearchPublicationSession,
    _publication_lease: SearchPublicationLease,
    pub(super) fulltext: PublishedSnapshotMetadata,
    vector: VectorSnapshotSummary,
    projections: Vec<ActiveSearchProjection>,
    projected_documents: Vec<ProjectedDocumentPlan>,
    vector_coverage: Vec<ActiveSearchProjection>,
}

impl PreparedSearchPublication<'_> {
    fn publication_session(&self) -> &SearchPublicationSession {
        self.publication_session
    }

    fn generation(&self) -> &str {
        self.fulltext.generation()
    }

    fn into_committed(self) -> CommittedSearchPublication {
        let Self {
            _publication_lease,
            fulltext,
            projections,
            ..
        } = self;
        CommittedSearchPublication {
            _publication_lease,
            fulltext,
            projections,
        }
    }
}

/// Borrowed decision surface for one validated publication transaction.
///
/// Callers may inspect the exact prepared artifacts and execute one metadata
/// decision, but cannot own or outlive the publication transaction.
pub(super) struct SearchPublicationView<'publication> {
    publication_session: &'publication SearchPublicationSession,
    fulltext: &'publication PublishedSnapshotMetadata,
    vector: &'publication VectorSnapshotSummary,
    projections: &'publication [ActiveSearchProjection],
    projected_documents: &'publication [ProjectedDocumentPlan],
    vector_coverage: &'publication [ActiveSearchProjection],
}

impl SearchPublicationView<'_> {
    pub(super) fn publication_session(&self) -> &SearchPublicationSession {
        self.publication_session
    }

    #[cfg(test)]
    pub(super) fn generation(&self) -> &str {
        self.fulltext.generation()
    }

    pub(super) fn fulltext(&self) -> &PublishedSnapshotMetadata {
        self.fulltext
    }

    pub(super) fn vector(&self) -> &VectorSnapshotSummary {
        self.vector
    }

    pub(super) fn projections(&self) -> &[ActiveSearchProjection] {
        self.projections
    }

    pub(super) fn projected_documents(&self) -> &[ProjectedDocumentPlan] {
        self.projected_documents
    }

    pub(super) fn vector_coverage(&self) -> &[ActiveSearchProjection] {
        self.vector_coverage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchPublicationDecision {
    Applied,
    NotApplied,
}

pub(super) enum SearchPublicationTransactionOutcome {
    Committed(CommittedSearchPublication),
    NotApplied,
}

impl SearchPublicationTransactionOutcome {
    pub(super) fn committed(&self) -> Option<&CommittedSearchPublication> {
        match self {
            Self::Committed(publication) => Some(publication),
            Self::NotApplied => None,
        }
    }

    pub(super) fn into_committed(self) -> Result<CommittedSearchPublication> {
        match self {
            Self::Committed(publication) => Ok(publication),
            Self::NotApplied => Err(ImportPipelineError::index_io()),
        }
    }
}

impl PreparedSearchPublication<'_> {
    fn view(&self) -> SearchPublicationView<'_> {
        SearchPublicationView {
            publication_session: self.publication_session,
            fulltext: &self.fulltext,
            vector: &self.vector,
            projections: &self.projections,
            projected_documents: &self.projected_documents,
            vector_coverage: &self.vector_coverage,
        }
    }

    fn terminate(
        self,
        now: UnixTimestamp,
        decision: Result<SearchPublicationDecision>,
    ) -> Result<SearchPublicationTransactionOutcome> {
        match decision {
            Ok(SearchPublicationDecision::Applied) => Ok(
                SearchPublicationTransactionOutcome::Committed(self.into_committed()),
            ),
            Ok(SearchPublicationDecision::NotApplied) => {
                let generation = self.generation().to_string();
                abandon_and_retire_search_publication(
                    self.publication_session(),
                    &generation,
                    now,
                    FailedGenerationArtifacts::both_published(),
                )?;
                Ok(SearchPublicationTransactionOutcome::NotApplied)
            }
            Err(primary) => {
                let generation = self.generation().to_string();
                abandon_and_retire_search_publication(
                    self.publication_session(),
                    &generation,
                    now,
                    FailedGenerationArtifacts::both_published(),
                )?;
                Err(primary)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProjectedDocumentPlan {
    RetainedUnchanged(ActiveSearchProjection),
    MetadataChanged(ActiveSearchProjection),
    Replacement(ActiveSearchProjection),
}

impl ProjectedDocumentPlan {
    pub(super) fn projection(&self) -> &ActiveSearchProjection {
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
    if state.publication.is_some() {
        return Err(ImportPipelineError::store_invariant());
    }
    let context = store
        .artifact_repair_context()
        .map_err(ImportPipelineError::store)?
        .ok_or_else(ImportPipelineError::store_invariant)?;
    if context.generation != generation || context.visible_epoch != state.visible_epoch {
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
    if context.projection_count != projections.len() as u64
        || context.projection_digest != projection_digest
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let vector_contract = match context.vector {
        ArtifactRepairVectorContext::Disabled => VectorModelContract::Disabled,
        ArtifactRepairVectorContext::Enabled {
            model_id,
            dimension,
        } => VectorModelContract::enabled(model_id, dimension as usize)
            .map_err(ImportPipelineError::vector)?,
    };

    Ok(SearchPublicationBase {
        generation: Some(generation),
        visible_epoch: state.visible_epoch,
        classifier_epoch: context.classifier_epoch,
        projections,
        vector_contract,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_search_publication_transaction(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    base: SearchPublicationBase,
    generation: &str,
    projections: Vec<ActiveSearchProjection>,
    staged_version_texts: &StagedSearchVersionTexts,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    write_fulltext: impl FnOnce() -> Result<PublishedSnapshotMetadata>,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> Result<SearchPublicationDecision>,
) -> Result<SearchPublicationTransactionOutcome> {
    let publication = prepare_search_publication(
        publication_session,
        now,
        classifier_epoch,
        base,
        generation,
        projections,
        staged_version_texts,
        vectorization,
        ensure_not_cancelled,
        write_fulltext,
    )?;
    let decision = decide(&publication.view());
    publication.terminate(now, decision)
}

#[cfg(test)]
pub(super) struct PreparedSearchPublicationForTest<'session>(PreparedSearchPublication<'session>);

#[cfg(test)]
impl PreparedSearchPublicationForTest<'_> {
    pub(super) fn generation(&self) -> &str {
        self.0.generation()
    }

    pub(super) fn fulltext(&self) -> &PublishedSnapshotMetadata {
        &self.0.fulltext
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_search_publication_for_test<'session>(
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
) -> Result<PreparedSearchPublicationForTest<'session>> {
    prepare_search_publication(
        publication_session,
        now,
        classifier_epoch,
        base,
        generation,
        projections,
        staged_version_texts,
        vectorization,
        ensure_not_cancelled,
        write_fulltext,
    )
    .map(PreparedSearchPublicationForTest)
}

#[cfg(test)]
pub(super) fn commit_prepared_search_publication_for_test(
    now: UnixTimestamp,
    publication: PreparedSearchPublicationForTest<'_>,
    documents: &[meta_store::Document],
) -> Result<CommittedSearchPublication> {
    let decision = super::search_publication_commit::decide_search_publication(
        &publication.0.view(),
        now,
        documents,
    );
    publication.0.terminate(now, decision)?.into_committed()
}

fn prepare_search_publication<'session>(
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
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
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

    let mut failed_artifacts = FailedGenerationArtifacts::default();
    let result = (|| {
        let fulltext = match write_fulltext() {
            Ok(fulltext) => {
                failed_artifacts.record_fulltext_published();
                fulltext
            }
            Err(error) => {
                failed_artifacts.record_fulltext_failure(&error);
                return Err(error);
            }
        };
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
        if fulltext.generation() != generation
            || fulltext.document_count() != projections.len()
            || fulltext.projection_digest() != &projection_digest
        {
            return Err(ImportPipelineError::store_invariant());
        }
        let vector = match publish_vector_generation(
            data_dir,
            store,
            generation,
            &base,
            &projections,
            staged_version_texts,
            vectorization,
            ensure_not_cancelled,
        ) {
            Ok(vector) => {
                failed_artifacts.record_vector_published();
                vector
            }
            Err(error) => {
                failed_artifacts.record_vector_failure(&error);
                return Err(error);
            }
        };
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
        let vector_coverage =
            validate_vector_publication(&vector, generation, &projections, &projection_digest)?;
        let fulltext_descriptor = meta_fulltext_descriptor(&fulltext)?;
        let vector_descriptor = meta_vector_descriptor(&vector)?;
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
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
    match result {
        Ok(publication) => Ok(publication),
        Err(primary) => {
            abandon_and_retire_search_publication(
                publication_session,
                generation,
                now,
                failed_artifacts,
            )?;
            Err(primary)
        }
    }
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
