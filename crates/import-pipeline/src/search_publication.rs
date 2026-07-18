use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use core_domain::VectorRecordId;
use index_fulltext::{IndexDocument, PublishedSnapshotMetadata};
use index_vector::{
    VectorDocument, VectorDocumentIdentity, VectorModelContract, VectorSnapshotRoot,
    VectorSnapshotStore, VectorSnapshotSummary, VectorSnapshotUpdate,
};
use meta_store::{
    ActiveSearchProjection, ContentDigest, Document, DocumentId, MetaStore, ResumeVersionId,
    SearchProjectionDigest, SearchProjectionServiceState, SearchPublicationCommit,
    SearchPublicationDraft, SearchPublicationOutcome, SearchPublicationValidation,
    TerminalDocumentUpdate, UnixTimestamp, VectorSnapshotDescriptor, VectorSnapshotMode,
};

use super::index_publication::SearchPublicationLock;
use super::{
    ImportPipelineError, Result, SearchPublicationEmbeddingFailure,
    SearchPublicationEmbeddingInput, SearchPublicationVectorization,
};

pub(super) struct PreparedSearchPublication {
    _publication_lock: SearchPublicationLock,
    pub(super) fulltext: PublishedSnapshotMetadata,
    pub(super) vector: VectorSnapshotSummary,
    pub(super) projections: Vec<ActiveSearchProjection>,
    vector_coverage: Vec<ActiveSearchProjection>,
}

#[must_use = "release the committed publication fence after dependent work is complete"]
pub(super) struct CommittedSearchPublication {
    _publication_lock: SearchPublicationLock,
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
            _publication_lock,
            fulltext,
            projections,
        } = self;
        drop(_publication_lock);
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

pub(super) fn load_search_publication_base(store: &MetaStore) -> Result<SearchPublicationBase> {
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if state.service_state == SearchProjectionServiceState::Repairing && state.generation.is_none()
    {
        return Ok(SearchPublicationBase {
            generation: None,
            visible_epoch: state.visible_epoch,
            classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
            projections: Vec::new(),
            vector_contract: VectorModelContract::Disabled,
        });
    }
    if state.service_state != SearchProjectionServiceState::Ready {
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

fn vector_model_contract(descriptor: &VectorSnapshotDescriptor) -> Result<VectorModelContract> {
    match descriptor.mode() {
        VectorSnapshotMode::Disabled => Ok(VectorModelContract::Disabled),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => VectorModelContract::enabled(model_id.clone(), *dimension as usize)
            .map_err(ImportPipelineError::vector),
    }
}

pub(super) fn prepare_search_publication(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    classifier_epoch: &str,
    publication_lock: SearchPublicationLock,
    base: SearchPublicationBase,
    generation: &str,
    projections: Vec<ActiveSearchProjection>,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    write_fulltext: impl FnOnce() -> Result<PublishedSnapshotMetadata>,
) -> Result<PreparedSearchPublication> {
    validate_classifier_epoch(&base, classifier_epoch)?;
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
    if store
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
            vectorization,
            ensure_not_cancelled,
        )?;
        let vector_coverage =
            validate_vector_publication(&vector, generation, &projections, &projection_digest)?;
        let fulltext_descriptor = meta_fulltext_descriptor(&fulltext)?;
        let vector_descriptor = meta_vector_descriptor(&vector)?;
        store
            .validate_search_publication(&SearchPublicationValidation {
                generation,
                fulltext: &fulltext_descriptor,
                vector: &vector_descriptor,
                now,
            })
            .map_err(ImportPipelineError::store)?;
        Ok(PreparedSearchPublication {
            _publication_lock: publication_lock,
            fulltext,
            vector,
            projections,
            vector_coverage,
        })
    })();
    if result.is_err() {
        store
            .abandon_search_publication(generation, now)
            .map_err(ImportPipelineError::store)?;
    }
    result
}

fn publish_vector_generation(
    data_dir: &Path,
    store: &MetaStore,
    generation: &str,
    base: &SearchPublicationBase,
    projections: &[ActiveSearchProjection],
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
) -> Result<VectorSnapshotSummary> {
    let target_contract = vectorization
        .vectorizer()
        .map(|vectorizer| {
            VectorModelContract::enabled(vectorizer.model_id(), vectorizer.dimension())
                .map_err(ImportPipelineError::vector)
        })
        .transpose()?
        .unwrap_or_else(|| base.vector_contract.clone());
    let vector_store =
        VectorSnapshotStore::new(data_dir.join("vector-index"), target_contract.clone())
            .map_err(ImportPipelineError::vector)?;
    if target_contract == VectorModelContract::Disabled {
        return vector_store
            .publish_generation(generation, projections.iter().cloned(), Vec::new())
            .map_err(ImportPipelineError::vector);
    }

    if base.generation.is_some() && base.vector_contract == target_contract {
        let root = VectorSnapshotRoot::new(data_dir.join("vector-index"));
        let base_reader = root.and_then(|root| {
            let lease = root.acquire_read_lease()?;
            root.open_generation_with_lease(
                base.generation
                    .as_deref()
                    .ok_or(index_vector::VectorIndexError::GenerationNotFound)?,
                &target_contract,
                lease,
            )
        });
        if let Ok(base_reader) = base_reader {
            let base_is_fully_covered = base_reader.summary().vector_document_count()
                == base.projections.len()
                && base_reader.summary().coverage_digest()
                    == base_reader.summary().projection_digest();
            if base_is_fully_covered {
                let replacements = changed_projections(&base.projections, projections);
                let replacement_vectors = embed_projections(
                    store,
                    &replacements,
                    &target_contract,
                    vectorization,
                    ensure_not_cancelled,
                )?;
                let update = VectorSnapshotUpdate::new(
                    projections.to_vec(),
                    replacement_vectors,
                    BTreeSet::new(),
                )
                .map_err(ImportPipelineError::vector)?;
                return vector_store
                    .publish_generation_from(base_reader, generation, update)
                    .map_err(ImportPipelineError::vector);
            }
        }
    }

    let vectors = embed_projections(
        store,
        projections,
        &target_contract,
        vectorization,
        ensure_not_cancelled,
    )?;
    vector_store
        .publish_generation(generation, projections.iter().cloned(), vectors)
        .map_err(ImportPipelineError::vector)
}

fn changed_projections(
    base: &[ActiveSearchProjection],
    target: &[ActiveSearchProjection],
) -> Vec<ActiveSearchProjection> {
    let base_versions = base
        .iter()
        .map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    target
        .iter()
        .filter(|projection| {
            base_versions.get(projection.document_id.as_str()).copied()
                != Some(projection.resume_version_id.as_str())
        })
        .cloned()
        .collect()
}

fn embed_projections(
    store: &MetaStore,
    projections: &[ActiveSearchProjection],
    contract: &VectorModelContract,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
) -> Result<Vec<VectorDocument>> {
    if projections.is_empty() {
        return Ok(Vec::new());
    }
    let vectorizer = vectorization
        .vectorizer()
        .ok_or_else(ImportPipelineError::vector_io)?;
    let (Some(model_id), Some(dimension)) = (contract.model_id(), contract.dimension()) else {
        return Err(ImportPipelineError::store_invariant());
    };
    if vectorizer.model_id() != model_id
        || vectorizer.dimension() != dimension
        || vectorizer.max_batch_inputs() == 0
        || vectorizer.max_text_bytes() == 0
    {
        return Err(ImportPipelineError::store_invariant());
    }

    let mut inputs = Vec::with_capacity(projections.len());
    for projection in projections {
        let version = store
            .resume_version_by_id(&projection.resume_version_id)
            .map_err(ImportPipelineError::store)?
            .filter(|version| version.document_id == projection.document_id)
            .ok_or_else(ImportPipelineError::store_invariant)?;
        let text = version
            .clean_text
            .as_deref()
            .or(version.raw_text.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(ImportPipelineError::store_invariant)?;
        if text.len() > vectorizer.max_text_bytes() {
            return Err(ImportPipelineError::vector_io());
        }
        inputs.push(SearchPublicationEmbeddingInput::new(
            projection.resume_version_id.to_string(),
            text,
        ));
    }

    let mut outputs = BTreeMap::new();
    for batch in inputs.chunks(vectorizer.max_batch_inputs()) {
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
        let cancelled = || ensure_not_cancelled.is_some_and(|check| check().is_err());
        let batch_outputs = vectorizer
            .embed_batch(batch, &cancelled)
            .map_err(|failure| match failure {
                SearchPublicationEmbeddingFailure::Cancelled => ImportPipelineError::cancelled(),
                SearchPublicationEmbeddingFailure::RuntimeUnavailable
                | SearchPublicationEmbeddingFailure::InvalidOutput => {
                    ImportPipelineError::vector_io()
                }
            })?;
        if batch_outputs.len() != batch.len() {
            return Err(ImportPipelineError::store_invariant());
        }
        for output in batch_outputs {
            if output.model_id() != model_id
                || output.values().len() != dimension
                || outputs.insert(output.id().to_string(), output).is_some()
            {
                return Err(ImportPipelineError::store_invariant());
            }
        }
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
    }

    projections
        .iter()
        .map(|projection| {
            let output = outputs
                .remove(projection.resume_version_id.as_str())
                .ok_or_else(ImportPipelineError::store_invariant)?;
            let vector_id = VectorRecordId::from_non_secret_parts(&[
                projection.resume_version_id.as_str(),
                model_id,
                "document",
            ]);
            let identity = VectorDocumentIdentity::new(
                vector_id.to_string(),
                projection.document_id.to_string(),
                projection.resume_version_id.to_string(),
                model_id,
            )
            .map_err(ImportPipelineError::vector)?;
            VectorDocument::new(identity, output.values().to_vec())
                .map_err(ImportPipelineError::vector)
        })
        .collect::<Result<Vec<_>>>()
        .and_then(|documents| {
            if outputs.is_empty() {
                Ok(documents)
            } else {
                Err(ImportPipelineError::store_invariant())
            }
        })
}

fn validate_vector_publication(
    vector: &VectorSnapshotSummary,
    generation: &str,
    projections: &[ActiveSearchProjection],
    projection_digest: &SearchProjectionDigest,
) -> Result<Vec<ActiveSearchProjection>> {
    if vector.generation() != generation
        || vector.projection_count() != projections.len()
        || vector.projection_digest() != projection_digest
    {
        return Err(ImportPipelineError::store_invariant());
    }
    match vector.model_contract() {
        VectorModelContract::Disabled
            if vector.vector_count() == 0
                && vector.vector_document_count() == 0
                && vector.coverage_digest()
                    == &SearchProjectionDigest::from_pairs::<_, &str, &str>([])
                        .map_err(|_| ImportPipelineError::store_invariant())? =>
        {
            Ok(Vec::new())
        }
        VectorModelContract::Enabled { .. }
            if vector.vector_count() >= projections.len()
                && vector.vector_document_count() == projections.len()
                && vector.coverage_digest() == projection_digest =>
        {
            Ok(projections.to_vec())
        }
        VectorModelContract::Disabled | VectorModelContract::Enabled { .. } => {
            Err(ImportPipelineError::store_invariant())
        }
    }
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
    store: &MetaStore,
    now: UnixTimestamp,
    publication: PreparedSearchPublication,
    documents: &[Document],
) -> Result<CommittedSearchPublication> {
    let PreparedSearchPublication {
        _publication_lock,
        fulltext,
        vector,
        projections,
        vector_coverage,
    } = publication;
    if fulltext.document_count() != projections.len()
        || vector.projection_count() != projections.len()
        || fulltext.generation() != vector.generation()
        || fulltext.generation().is_empty()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let terminal_documents = terminal_document_updates(store, documents)?;
    let commit = SearchPublicationCommit {
        generation: fulltext.generation(),
        terminal_documents: &terminal_documents,
        projections: &projections,
        vector_coverage: &vector_coverage,
        now,
    };
    match store
        .commit_search_publication(&commit)
        .map_err(ImportPipelineError::store)?
    {
        SearchPublicationOutcome::Applied => Ok(CommittedSearchPublication {
            _publication_lock,
            fulltext,
            projections,
        }),
        SearchPublicationOutcome::Superseded => Err(ImportPipelineError::index_io()),
    }
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

fn meta_vector_descriptor(summary: &VectorSnapshotSummary) -> Result<VectorSnapshotDescriptor> {
    match summary.model_contract() {
        VectorModelContract::Disabled => Ok(VectorSnapshotDescriptor::disabled(
            summary.generation().to_string(),
            u64::try_from(summary.projection_count())
                .map_err(|_| ImportPipelineError::store_invariant())?,
            summary.projection_digest().clone(),
            summary.coverage_digest().clone(),
            summary.logical_content_digest().clone(),
        )),
        VectorModelContract::Enabled {
            model_id,
            dimension,
        } => Ok(VectorSnapshotDescriptor::enabled(
            meta_store::EnabledVectorSnapshotDescriptor {
                generation: summary.generation().to_string(),
                model_id: model_id.clone(),
                dimension: u32::try_from(*dimension)
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                projection_count: u64::try_from(summary.projection_count())
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                projection_digest: summary.projection_digest().clone(),
                coverage_digest: summary.coverage_digest().clone(),
                vector_count: u64::try_from(summary.vector_count())
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                document_count: u64::try_from(summary.vector_document_count())
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                resume_version_count: u64::try_from(summary.vector_document_count())
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                logical_content_digest: summary.logical_content_digest().clone(),
            },
        )),
    }
}

fn terminal_document_updates(
    store: &MetaStore,
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
