use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::model_contract::VectorModelContract;
use crate::publish_control::VectorSnapshotPublishControl;
use crate::snapshot_model::validate_projection_with_control;
use core_domain::ActiveSearchProjection;

pub(crate) const MAX_MODEL_ID_CHARS: usize = 128;
const STABLE_ID_DIGEST_LEN: usize = 32;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorDocumentIdentity {
    vector_id: String,
    document_id: String,
    resume_version_id: String,
    model_id: String,
}

impl VectorDocumentIdentity {
    pub fn new(
        vector_id: impl Into<String>,
        document_id: impl Into<String>,
        resume_version_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, VectorIndexError> {
        let vector_id = vector_id.into();
        let document_id = document_id.into();
        let resume_version_id = resume_version_id.into();
        let model_id = model_id.into();
        validate_stable_id(&vector_id, "vec_")?;
        validate_stable_id(&document_id, "doc_")?;
        validate_stable_id(&resume_version_id, "ver_")?;
        validate_model_id(&model_id)?;
        Ok(Self {
            vector_id,
            document_id,
            resume_version_id,
            model_id,
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct VectorDocument {
    identity: VectorDocumentIdentity,
    values: Vec<f32>,
}

impl VectorDocument {
    pub fn new(
        identity: VectorDocumentIdentity,
        values: Vec<f32>,
    ) -> Result<Self, VectorIndexError> {
        validate_values(&values)?;
        Ok(Self { identity, values })
    }

    pub fn identity(&self) -> &VectorDocumentIdentity {
        &self.identity
    }

    pub fn vector_id(&self) -> &str {
        &self.identity.vector_id
    }

    pub fn document_id(&self) -> &str {
        &self.identity.document_id
    }

    pub fn resume_version_id(&self) -> &str {
        &self.identity.resume_version_id
    }

    pub fn model_id(&self) -> &str {
        &self.identity.model_id
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for VectorDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorDocument")
            .field("vector_id", &self.vector_id())
            .field("document_id", &self.document_id())
            .field("resume_version_id", &self.resume_version_id())
            .field("model_id", &self.model_id())
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct QueryVector {
    values: Vec<f32>,
}

impl QueryVector {
    pub fn new(values: Vec<f32>) -> Result<Self, VectorIndexError> {
        validate_values(&values)?;
        Ok(Self { values })
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for QueryVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QueryVector")
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct VectorHit {
    vector_id: String,
    document_id: String,
    resume_version_id: String,
    model_id: String,
    score: f32,
}

impl VectorHit {
    pub(crate) fn from_document(document: &VectorDocument, score: f32) -> Self {
        Self {
            vector_id: document.vector_id().to_string(),
            document_id: document.document_id().to_string(),
            resume_version_id: document.resume_version_id().to_string(),
            model_id: document.model_id().to_string(),
            score,
        }
    }

    pub fn vector_id(&self) -> &str {
        &self.vector_id
    }

    pub fn document_id(&self) -> &str {
        &self.document_id
    }

    pub fn resume_version_id(&self) -> &str {
        &self.resume_version_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn score(&self) -> f32 {
        self.score
    }
}

impl fmt::Debug for VectorHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorHit")
            .field("vector_id", &self.vector_id)
            .field("document_id", &self.document_id)
            .field("resume_version_id", &self.resume_version_id)
            .field("model_id", &self.model_id)
            .field("score", &self.score)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorIndexError {
    Cancelled,
    PublicationBusy,
    InvalidDimension { expected: usize, actual: usize },
    InvalidVectorValue,
    InvalidModelId,
    InvalidIdentity,
    InvalidGeneration,
    InvalidModelContract,
    SemanticUnavailable,
    PublicationProjectionMismatch,
    DuplicateVectorId,
    ConflictingDocumentVersion,
    GenerationAlreadyExists,
    GenerationNotFound,
    LeaseRootMismatch,
    SchemaMismatch,
    CorruptSnapshot,
    StorageLayoutInvalid,
    Storage,
}

impl fmt::Display for VectorIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("vector snapshot publication cancelled"),
            Self::PublicationBusy => formatter.write_str("vector snapshot publication is busy"),
            Self::InvalidDimension { expected, actual } => {
                write!(
                    formatter,
                    "vector dimension must be {expected}, got {actual}"
                )
            }
            Self::InvalidVectorValue => formatter.write_str("vector values must be finite"),
            Self::InvalidModelId => formatter.write_str("vector model id is invalid"),
            Self::InvalidIdentity => formatter.write_str("vector identity is invalid"),
            Self::InvalidGeneration => formatter.write_str("vector generation is invalid"),
            Self::InvalidModelContract => formatter.write_str("vector model contract is invalid"),
            Self::SemanticUnavailable => formatter.write_str("semantic search is unavailable"),
            Self::PublicationProjectionMismatch => {
                formatter.write_str("vector publication does not match the active projection")
            }
            Self::DuplicateVectorId => formatter.write_str("vector id is duplicated"),
            Self::ConflictingDocumentVersion => {
                formatter.write_str("document has conflicting resume versions")
            }
            Self::GenerationAlreadyExists => {
                formatter.write_str("vector generation already exists")
            }
            Self::GenerationNotFound => formatter.write_str("vector generation is unavailable"),
            Self::LeaseRootMismatch => {
                formatter.write_str("vector read lease belongs to another store")
            }
            Self::SchemaMismatch => formatter.write_str("vector snapshot schema is incompatible"),
            Self::CorruptSnapshot => formatter.write_str("vector snapshot is corrupt"),
            Self::StorageLayoutInvalid => formatter.write_str("vector storage layout is invalid"),
            Self::Storage => formatter.write_str("vector storage is unavailable"),
        }
    }
}

impl std::error::Error for VectorIndexError {}

pub(crate) fn validate_documents_with_control(
    model_contract: &VectorModelContract,
    projection: &[ActiveSearchProjection],
    documents: &[VectorDocument],
    control: VectorSnapshotPublishControl<'_>,
) -> Result<(), VectorIndexError> {
    control.check()?;
    model_contract.validate()?;
    let active_versions = validate_projection_with_control(projection, control)?;
    if matches!(model_contract, VectorModelContract::Disabled) && !documents.is_empty() {
        return Err(VectorIndexError::InvalidModelContract);
    }
    let mut vector_ids = BTreeSet::new();
    let mut document_versions = BTreeMap::new();
    for (index, document) in documents.iter().enumerate() {
        let VectorModelContract::Enabled {
            model_id,
            dimension,
        } = model_contract
        else {
            return Err(VectorIndexError::InvalidModelContract);
        };
        validate_dimension(*dimension, document.values())?;
        validate_values(document.values())?;
        if document.model_id() != model_id {
            return Err(VectorIndexError::InvalidModelContract);
        }
        if active_versions
            .get(document.document_id())
            .map(String::as_str)
            != Some(document.resume_version_id())
        {
            return Err(VectorIndexError::PublicationProjectionMismatch);
        }
        if !vector_ids.insert(document.vector_id()) {
            return Err(VectorIndexError::DuplicateVectorId);
        }
        match document_versions.insert(document.document_id(), document.resume_version_id()) {
            Some(version) if version != document.resume_version_id() => {
                return Err(VectorIndexError::ConflictingDocumentVersion);
            }
            _ => {}
        }
        control.check_after_record(index + 1)?;
    }
    control.check()?;
    Ok(())
}

pub(crate) fn validate_dimension(expected: usize, values: &[f32]) -> Result<(), VectorIndexError> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(VectorIndexError::InvalidDimension {
            expected,
            actual: values.len(),
        })
    }
}

pub(crate) fn validate_model_id(model_id: &str) -> Result<(), VectorIndexError> {
    let char_count = model_id.chars().count();
    if char_count == 0 || char_count > MAX_MODEL_ID_CHARS || model_id.chars().any(char::is_control)
    {
        Err(VectorIndexError::InvalidModelId)
    } else {
        Ok(())
    }
}

fn validate_stable_id(value: &str, prefix: &str) -> Result<(), VectorIndexError> {
    let Some(digest) = value.strip_prefix(prefix) else {
        return Err(VectorIndexError::InvalidIdentity);
    };
    if digest.len() == STABLE_ID_DIGEST_LEN
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(VectorIndexError::InvalidIdentity)
    }
}

pub(crate) fn validate_vector_id(value: &str) -> Result<(), VectorIndexError> {
    validate_stable_id(value, "vec_")
}

fn validate_values(values: &[f32]) -> Result<(), VectorIndexError> {
    if values.is_empty() {
        return Err(VectorIndexError::InvalidDimension {
            expected: 1,
            actual: 0,
        });
    }
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(VectorIndexError::InvalidVectorValue)
    }
}
