pub fn crate_name() -> &'static str {
    "index-vector"
}

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

pub trait VectorIndex {
    fn upsert(&self, vectors: Vec<VectorDocument>) -> Result<(), VectorIndexError>;
    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError>;
    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError>;
    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError>;
}

#[derive(Debug)]
pub struct InMemoryVectorIndex {
    dimension: usize,
    state: Mutex<IndexState>,
}

impl InMemoryVectorIndex {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            state: Mutex::new(IndexState::default()),
        }
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&self, vectors: Vec<VectorDocument>) -> Result<(), VectorIndexError> {
        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector in vectors {
            validate_dimension(self.dimension, vector.values())?;
            state.deleted.remove(vector.vector_id());
            state.vectors.insert(vector.vector_id().to_string(), vector);
        }
        Ok(())
    }

    fn mark_deleted(&self, vector_ids: &[&str]) -> Result<(), VectorIndexError> {
        let mut state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        for vector_id in vector_ids {
            state.deleted.insert((*vector_id).to_string());
        }
        Ok(())
    }

    fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, query.values())?;
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        let mut hits = state
            .vectors
            .values()
            .filter(|vector| !state.deleted.contains(vector.vector_id()))
            .map(|vector| {
                VectorHit::new(
                    vector.vector_id().to_string(),
                    vector.doc_id().to_string(),
                    cosine_similarity(query.values(), vector.values()),
                )
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score()
                .partial_cmp(&left.score())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.doc_id().cmp(right.doc_id()))
        });
        hits.truncate(k);
        Ok(hits)
    }

    fn snapshot(&self) -> Result<VectorSnapshot, VectorIndexError> {
        let state = self.state.lock().map_err(|_| VectorIndexError::Poisoned)?;
        Ok(VectorSnapshot {
            vector_count: state.vectors.len(),
            deleted_count: state.deleted.len(),
            dimension: self.dimension,
        })
    }
}

#[derive(Default, Debug)]
struct IndexState {
    vectors: BTreeMap<String, VectorDocument>,
    deleted: BTreeSet<String>,
}

#[derive(Clone, PartialEq)]
pub struct VectorDocument {
    vector_id: String,
    doc_id: String,
    values: Vec<f32>,
}

impl VectorDocument {
    pub fn new(
        vector_id: impl Into<String>,
        doc_id: impl Into<String>,
        values: Vec<f32>,
    ) -> Result<Self, VectorIndexError> {
        if values.is_empty() {
            return Err(VectorIndexError::InvalidDimension {
                expected: 1,
                actual: 0,
            });
        }

        Ok(Self {
            vector_id: vector_id.into(),
            doc_id: doc_id.into(),
            values,
        })
    }

    pub fn vector_id(&self) -> &str {
        &self.vector_id
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for VectorDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorDocument")
            .field("vector_id", &self.vector_id)
            .field("doc_id", &self.doc_id)
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
        if values.is_empty() {
            return Err(VectorIndexError::InvalidDimension {
                expected: 1,
                actual: 0,
            });
        }

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
    doc_id: String,
    score: f32,
}

impl VectorHit {
    fn new(vector_id: String, doc_id: String, score: f32) -> Self {
        Self {
            vector_id,
            doc_id,
            score,
        }
    }

    pub fn vector_id(&self) -> &str {
        &self.vector_id
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
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
            .field("doc_id", &self.doc_id)
            .field("score", &self.score)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorSnapshot {
    vector_count: usize,
    deleted_count: usize,
    dimension: usize,
}

impl VectorSnapshot {
    pub fn vector_count(self) -> usize {
        self.vector_count
    }

    pub fn deleted_count(self) -> usize {
        self.deleted_count
    }

    pub fn dimension(self) -> usize {
        self.dimension
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VectorIndexError {
    InvalidDimension { expected: usize, actual: usize },
    Poisoned,
}

impl fmt::Display for VectorIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimension { expected, actual } => write!(
                formatter,
                "vector dimension must be {expected}, got {actual}"
            ),
            Self::Poisoned => formatter.write_str("vector index state is unavailable"),
        }
    }
}

impl std::error::Error for VectorIndexError {}

fn validate_dimension(expected: usize, values: &[f32]) -> Result<(), VectorIndexError> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(VectorIndexError::InvalidDimension {
            expected,
            actual: values.len(),
        })
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}
