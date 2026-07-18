use std::collections::{BTreeMap, BTreeSet};

use core_domain::{
    ActiveSearchProjection, ContentDigest, SearchProjectionDigest, SearchProjectionDigestError,
};

use crate::model::{validate_vector_id, VectorDocument, VectorIndexError};
use crate::model_contract::VectorModelContract;

const SNAPSHOT_MANIFEST_SCHEMA_V3: &str = "vector.snapshot.v3";
const VECTOR_INDEX_SCHEMA_V3: &str = "hnsw-vector.v3";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorSnapshotSchema {
    manifest_schema: &'static str,
    index_schema: &'static str,
}

impl VectorSnapshotSchema {
    pub const fn manifest_schema(self) -> &'static str {
        self.manifest_schema
    }

    pub const fn index_schema(self) -> &'static str {
        self.index_schema
    }
}

pub const VECTOR_SNAPSHOT_SCHEMA_V3: VectorSnapshotSchema = VectorSnapshotSchema {
    manifest_schema: SNAPSHOT_MANIFEST_SCHEMA_V3,
    index_schema: VECTOR_INDEX_SCHEMA_V3,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VectorSnapshotDigests {
    projection: SearchProjectionDigest,
    coverage: SearchProjectionDigest,
    logical_content: ContentDigest,
    artifact: ContentDigest,
}

impl VectorSnapshotDigests {
    pub(crate) fn new(
        projection: SearchProjectionDigest,
        coverage: SearchProjectionDigest,
        logical_content: ContentDigest,
        artifact: ContentDigest,
    ) -> Self {
        Self {
            projection,
            coverage,
            logical_content,
            artifact,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorSnapshotSummary {
    generation: String,
    model_contract: VectorModelContract,
    vector_count: usize,
    projection_count: usize,
    vector_document_count: usize,
    projection_digest: SearchProjectionDigest,
    coverage_digest: SearchProjectionDigest,
    logical_content_digest: ContentDigest,
    artifact_digest: ContentDigest,
}

impl VectorSnapshotSummary {
    pub(crate) fn from_contents(
        generation: String,
        model_contract: VectorModelContract,
        projection: &[ActiveSearchProjection],
        documents: &[VectorDocument],
        digests: VectorSnapshotDigests,
    ) -> Self {
        Self {
            generation,
            model_contract,
            vector_count: documents.len(),
            projection_count: projection.len(),
            vector_document_count: documents
                .iter()
                .map(VectorDocument::document_id)
                .collect::<BTreeSet<_>>()
                .len(),
            projection_digest: digests.projection,
            coverage_digest: digests.coverage,
            logical_content_digest: digests.logical_content,
            artifact_digest: digests.artifact,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub const fn schema(&self) -> VectorSnapshotSchema {
        VECTOR_SNAPSHOT_SCHEMA_V3
    }

    pub fn vector_count(&self) -> usize {
        self.vector_count
    }

    pub fn projection_count(&self) -> usize {
        self.projection_count
    }

    pub fn vector_document_count(&self) -> usize {
        self.vector_document_count
    }

    pub fn model_contract(&self) -> &VectorModelContract {
        &self.model_contract
    }

    pub fn projection_digest(&self) -> &SearchProjectionDigest {
        &self.projection_digest
    }

    pub fn coverage_digest(&self) -> &SearchProjectionDigest {
        &self.coverage_digest
    }

    pub fn logical_content_digest(&self) -> &ContentDigest {
        &self.logical_content_digest
    }

    pub fn artifact_digest(&self) -> &ContentDigest {
        &self.artifact_digest
    }
}

/// Typed input for deriving a complete immutable generation from one exact
/// base reader.
#[derive(Clone, Debug)]
pub struct VectorSnapshotUpdate {
    active_projection: Vec<ActiveSearchProjection>,
    replacement_documents: Vec<VectorDocument>,
    removed_vector_ids: BTreeSet<String>,
}

impl VectorSnapshotUpdate {
    pub fn new(
        active_projection: Vec<ActiveSearchProjection>,
        replacement_documents: Vec<VectorDocument>,
        removed_vector_ids: BTreeSet<String>,
    ) -> Result<Self, VectorIndexError> {
        let active_versions = validate_projection(&active_projection)?;
        for vector_id in &removed_vector_ids {
            validate_vector_id(vector_id)?;
        }
        for document in &replacement_documents {
            if active_versions
                .get(document.document_id())
                .map(String::as_str)
                != Some(document.resume_version_id())
            {
                return Err(VectorIndexError::PublicationProjectionMismatch);
            }
        }
        Ok(Self {
            active_projection,
            replacement_documents,
            removed_vector_ids,
        })
    }

    pub(crate) fn apply(
        self,
        base: &[VectorDocument],
    ) -> (Vec<ActiveSearchProjection>, Vec<VectorDocument>) {
        let active_versions = self
            .active_projection
            .iter()
            .map(|entry| {
                (
                    entry.document_id.as_str().to_string(),
                    entry.resume_version_id.as_str().to_string(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let replacement_ids = self
            .replacement_documents
            .iter()
            .map(VectorDocument::vector_id)
            .collect::<BTreeSet<_>>();
        let mut documents = base
            .iter()
            .filter(|document| {
                active_versions
                    .get(document.document_id())
                    .is_some_and(|version| version == document.resume_version_id())
            })
            .filter(|document| !self.removed_vector_ids.contains(document.vector_id()))
            .filter(|document| !replacement_ids.contains(document.vector_id()))
            .cloned()
            .collect::<Vec<_>>();
        documents.extend(self.replacement_documents);
        (self.active_projection, documents)
    }
}

pub(crate) fn validate_projection(
    projection: &[ActiveSearchProjection],
) -> Result<BTreeMap<String, String>, VectorIndexError> {
    let pairs = projection
        .iter()
        .map(|entry| {
            (
                entry.document_id.as_str().to_string(),
                entry.resume_version_id.as_str().to_string(),
            )
        })
        .collect::<Vec<_>>();
    SearchProjectionDigest::from_pairs(
        pairs
            .iter()
            .map(|(document_id, version_id)| (document_id.as_str(), version_id.as_str())),
    )
    .map_err(map_projection_digest_error)?;
    Ok(pairs.into_iter().collect())
}

pub(crate) fn projection_digest(
    projection: &[ActiveSearchProjection],
) -> Result<SearchProjectionDigest, VectorIndexError> {
    SearchProjectionDigest::from_pairs(
        projection
            .iter()
            .map(|entry| (entry.document_id.as_str(), entry.resume_version_id.as_str())),
    )
    .map_err(map_projection_digest_error)
}

fn map_projection_digest_error(_error: SearchProjectionDigestError) -> VectorIndexError {
    VectorIndexError::PublicationProjectionMismatch
}
