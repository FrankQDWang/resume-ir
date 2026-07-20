use std::collections::{BTreeMap, BTreeSet};

use core_domain::{
    ActiveSearchProjection, ContentDigest, SearchProjectionDigest, SearchProjectionDigestError,
};

use crate::model::{validate_vector_id, VectorDocument, VectorIndexError};
use crate::model_contract::VectorModelContract;
use crate::publish_control::VectorSnapshotPublishControl;

const SNAPSHOT_MANIFEST_SCHEMA_V4: &str = "vector.snapshot.v4";
const VECTOR_INDEX_SCHEMA_V4: &str = "hnsw-vector.v4";

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

pub const VECTOR_SNAPSHOT_SCHEMA_V4: VectorSnapshotSchema = VectorSnapshotSchema {
    manifest_schema: SNAPSHOT_MANIFEST_SCHEMA_V4,
    index_schema: VECTOR_INDEX_SCHEMA_V4,
};

/// Bounded metadata validated directly from one immutable generation manifest.
///
/// This type does not imply that the encrypted payload was decoded or that an
/// ANN index was constructed. Exact readers perform those stronger checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorSnapshotManifestMetadata {
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

impl VectorSnapshotManifestMetadata {
    pub(crate) fn new(
        generation: String,
        model_contract: VectorModelContract,
        vector_count: usize,
        projection_count: usize,
        vector_document_count: usize,
        digests: VectorSnapshotDigests,
    ) -> Self {
        Self {
            generation,
            model_contract,
            vector_count,
            projection_count,
            vector_document_count,
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
        VECTOR_SNAPSHOT_SCHEMA_V4
    }

    pub fn model_contract(&self) -> &VectorModelContract {
        &self.model_contract
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
    pub(crate) fn from_contents_with_control(
        generation: String,
        model_contract: VectorModelContract,
        projection: &[ActiveSearchProjection],
        documents: &[VectorDocument],
        digests: VectorSnapshotDigests,
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<Self, VectorIndexError> {
        control.check()?;
        let mut vector_documents = BTreeSet::new();
        for (index, document) in documents.iter().enumerate() {
            vector_documents.insert(document.document_id());
            control.check_after_record(index + 1)?;
        }
        control.check()?;
        Ok(Self {
            generation,
            model_contract,
            vector_count: documents.len(),
            projection_count: projection.len(),
            vector_document_count: vector_documents.len(),
            projection_digest: digests.projection,
            coverage_digest: digests.coverage,
            logical_content_digest: digests.logical_content,
            artifact_digest: digests.artifact,
        })
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub const fn schema(&self) -> VectorSnapshotSchema {
        VECTOR_SNAPSHOT_SCHEMA_V4
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

    pub(crate) fn apply_with_control(
        self,
        base: &[VectorDocument],
        control: VectorSnapshotPublishControl<'_>,
    ) -> Result<(Vec<ActiveSearchProjection>, Vec<VectorDocument>), VectorIndexError> {
        control.check()?;
        let mut active_versions = BTreeMap::new();
        for (index, entry) in self.active_projection.iter().enumerate() {
            active_versions.insert(
                entry.document_id.as_str().to_string(),
                entry.resume_version_id.as_str().to_string(),
            );
            control.check_after_record(index + 1)?;
        }
        let mut replacement_ids = BTreeSet::new();
        for (index, document) in self.replacement_documents.iter().enumerate() {
            replacement_ids.insert(document.vector_id());
            control.check_after_record(index + 1)?;
        }
        let mut documents =
            Vec::with_capacity(base.len().saturating_add(self.replacement_documents.len()));
        for (index, document) in base.iter().enumerate() {
            if active_versions
                .get(document.document_id())
                .is_some_and(|version| version == document.resume_version_id())
                && !self.removed_vector_ids.contains(document.vector_id())
                && !replacement_ids.contains(document.vector_id())
            {
                documents.push(document.clone());
            }
            control.check_after_record(index + 1)?;
        }
        for (index, document) in self.replacement_documents.into_iter().enumerate() {
            documents.push(document);
            control.check_after_record(index + 1)?;
        }
        control.check()?;
        Ok((self.active_projection, documents))
    }
}

pub(crate) fn validate_projection(
    projection: &[ActiveSearchProjection],
) -> Result<BTreeMap<String, String>, VectorIndexError> {
    validate_projection_with_control(projection, VectorSnapshotPublishControl::disabled())
}

pub(crate) fn validate_projection_with_control(
    projection: &[ActiveSearchProjection],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<BTreeMap<String, String>, VectorIndexError> {
    control.check()?;
    let mut pairs = Vec::with_capacity(projection.len());
    for (index, entry) in projection.iter().enumerate() {
        pairs.push((
            entry.document_id.as_str().to_string(),
            entry.resume_version_id.as_str().to_string(),
        ));
        control.check_after_record(index + 1)?;
    }
    SearchProjectionDigest::from_pairs(
        pairs
            .iter()
            .map(|(document_id, version_id)| (document_id.as_str(), version_id.as_str())),
    )
    .map_err(map_projection_digest_error)?;
    control.check()?;
    let mut validated = BTreeMap::new();
    for (index, (document_id, version_id)) in pairs.into_iter().enumerate() {
        validated.insert(document_id, version_id);
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    Ok(validated)
}

pub(crate) fn projection_digest_with_control(
    projection: &[ActiveSearchProjection],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<SearchProjectionDigest, VectorIndexError> {
    control.check()?;
    let mut pairs = Vec::with_capacity(projection.len());
    for (index, entry) in projection.iter().enumerate() {
        pairs.push((entry.document_id.as_str(), entry.resume_version_id.as_str()));
        control.check_after_record(index + 1)?;
    }
    let digest = SearchProjectionDigest::from_pairs(pairs).map_err(map_projection_digest_error)?;
    control.check()?;
    Ok(digest)
}

fn map_projection_digest_error(_error: SearchProjectionDigestError) -> VectorIndexError {
    VectorIndexError::PublicationProjectionMismatch
}
