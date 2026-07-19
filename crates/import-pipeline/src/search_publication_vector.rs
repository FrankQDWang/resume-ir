use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use core_domain::VectorRecordId;
use index_fulltext::IndexDocument;
use index_vector::{
    VectorDocument, VectorDocumentIdentity, VectorModelContract, VectorSnapshotRoot,
    VectorSnapshotStore, VectorSnapshotSummary, VectorSnapshotUpdate,
};
use meta_store::{
    ActiveSearchProjection, DocumentId, OwnedMetaStore, ResumeVersionId, SearchProjectionDigest,
    VectorSnapshotDescriptor, VectorSnapshotMode,
};

use super::search_publication::SearchPublicationBase;
use super::{
    ImportPipelineError, Result, SearchPublicationEmbeddingFailure,
    SearchPublicationEmbeddingInput, SearchPublicationVectorization,
};

pub(super) struct StagedSearchVersionText {
    pub(super) document_id: DocumentId,
    text: String,
}

pub(super) type StagedSearchVersionTexts = BTreeMap<String, StagedSearchVersionText>;

pub(super) fn staged_search_version_texts(
    documents: &[IndexDocument],
) -> Result<StagedSearchVersionTexts> {
    let mut staged = BTreeMap::new();
    for document in documents {
        let document_id = document
            .doc_id
            .parse::<DocumentId>()
            .map_err(|_| ImportPipelineError::store_invariant())?;
        let version_id = document
            .resume_version_id
            .parse::<ResumeVersionId>()
            .map_err(|_| ImportPipelineError::store_invariant())?;
        if document.clean_text.trim().is_empty()
            || staged
                .insert(
                    version_id.to_string(),
                    StagedSearchVersionText {
                        document_id,
                        text: document.clean_text.clone(),
                    },
                )
                .is_some()
        {
            return Err(ImportPipelineError::store_invariant());
        }
    }
    Ok(staged)
}

pub(super) fn vector_model_contract(
    descriptor: &VectorSnapshotDescriptor,
) -> Result<VectorModelContract> {
    match descriptor.mode() {
        VectorSnapshotMode::Disabled => Ok(VectorModelContract::Disabled),
        VectorSnapshotMode::Enabled {
            model_id,
            dimension,
        } => VectorModelContract::enabled(model_id.clone(), *dimension as usize)
            .map_err(ImportPipelineError::vector),
    }
}

pub(super) fn publish_vector_generation(
    data_dir: &Path,
    store: &OwnedMetaStore,
    generation: &str,
    base: &SearchPublicationBase,
    projections: &[ActiveSearchProjection],
    staged_version_texts: &StagedSearchVersionTexts,
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
                    staged_version_texts,
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
        staged_version_texts,
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
    store: &OwnedMetaStore,
    projections: &[ActiveSearchProjection],
    staged_version_texts: &StagedSearchVersionTexts,
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
        let text =
            if let Some(staged) = staged_version_texts.get(projection.resume_version_id.as_str()) {
                if staged.document_id != projection.document_id {
                    return Err(ImportPipelineError::store_invariant());
                }
                staged.text.trim().to_string()
            } else {
                let version = store
                    .resume_version_by_id(&projection.resume_version_id)
                    .map_err(ImportPipelineError::store)?
                    .filter(|version| version.document_id == projection.document_id)
                    .ok_or_else(ImportPipelineError::store_invariant)?;
                version
                    .clean_text
                    .as_deref()
                    .or(version.raw_text.as_deref())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .ok_or_else(ImportPipelineError::store_invariant)?
                    .to_string()
            };
        if text.is_empty() {
            return Err(ImportPipelineError::store_invariant());
        }
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

pub(super) fn validate_vector_publication(
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

pub(super) fn meta_vector_descriptor(
    summary: &VectorSnapshotSummary,
) -> Result<VectorSnapshotDescriptor> {
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
