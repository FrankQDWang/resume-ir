//! Vector index interfaces for the semantic retrieval skeleton.

use embedder::EmbeddingVector;
use std::fmt;
use thiserror::Error;

/// Supported vector similarity metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimilarityMetric {
    /// Cosine similarity over dense vectors.
    Cosine,
    /// Dot product over dense vectors.
    Dot,
}

/// One document vector stored in a vector index.
#[derive(Clone, PartialEq)]
pub struct VectorDocument {
    doc_id: String,
    vector: EmbeddingVector,
}

impl VectorDocument {
    /// Creates a vector document.
    #[must_use]
    pub fn new(doc_id: impl Into<String>, vector: EmbeddingVector) -> Self {
        Self {
            doc_id: doc_id.into(),
            vector,
        }
    }

    /// Returns the document identifier.
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    /// Returns the stored vector.
    #[must_use]
    pub fn vector(&self) -> &EmbeddingVector {
        &self.vector
    }
}

impl fmt::Debug for VectorDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorDocument")
            .field("doc_id", &self.doc_id)
            .field("vector_dimension", &self.vector.dimension())
            .field("vector", &"[redacted vector payload]")
            .finish()
    }
}

/// Query vector and search options.
#[derive(Clone, PartialEq)]
pub struct VectorSearchRequest {
    query: EmbeddingVector,
    metric: SimilarityMetric,
    top_k: usize,
}

impl VectorSearchRequest {
    /// Creates a vector search request.
    pub fn new(
        query: EmbeddingVector,
        metric: SimilarityMetric,
        top_k: usize,
    ) -> Result<Self, VectorIndexError> {
        if top_k == 0 {
            return Err(VectorIndexError::InvalidTopK { top_k });
        }
        Ok(Self {
            query,
            metric,
            top_k,
        })
    }

    /// Returns the query vector.
    #[must_use]
    pub fn query(&self) -> &EmbeddingVector {
        &self.query
    }

    /// Returns the similarity metric.
    #[must_use]
    pub fn metric(&self) -> SimilarityMetric {
        self.metric
    }

    /// Returns the maximum number of hits to return.
    #[must_use]
    pub fn top_k(&self) -> usize {
        self.top_k
    }
}

impl fmt::Debug for VectorSearchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorSearchRequest")
            .field("vector_dimension", &self.query.dimension())
            .field("query", &"[redacted query vector]")
            .field("metric", &self.metric)
            .field("top_k", &self.top_k)
            .finish()
    }
}

/// Ranked vector search hit.
#[derive(Clone, PartialEq)]
pub struct VectorHit {
    doc_id: String,
    score: f32,
}

impl VectorHit {
    /// Creates a vector search hit.
    #[must_use]
    pub fn new(doc_id: impl Into<String>, score: f32) -> Self {
        Self {
            doc_id: doc_id.into(),
            score,
        }
    }

    /// Returns the document identifier.
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    /// Returns the similarity score.
    #[must_use]
    pub fn score(&self) -> f32 {
        self.score
    }
}

impl fmt::Debug for VectorHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorHit")
            .field("doc_id", &self.doc_id)
            .field("score", &self.score)
            .finish()
    }
}

/// Replaceable vector index interface.
pub trait VectorIndex {
    /// Inserts or replaces a document vector.
    fn upsert(&mut self, document: VectorDocument) -> Result<(), VectorIndexError>;

    /// Marks a document as deleted so future searches ignore it.
    fn mark_deleted(&mut self, doc_id: &str) -> Result<(), VectorIndexError>;

    /// Searches for nearest vectors according to the request metric.
    fn search(&self, request: &VectorSearchRequest) -> Result<Vec<VectorHit>, VectorIndexError>;
}

/// Deterministic in-memory vector index for tests.
#[derive(Clone, PartialEq)]
pub struct InMemoryVectorIndex {
    dimension: usize,
    documents: Vec<StoredVectorDocument>,
}

impl InMemoryVectorIndex {
    /// Creates an empty in-memory index for a fixed vector dimension.
    pub fn new(dimension: usize) -> Result<Self, VectorIndexError> {
        if dimension == 0 {
            return Err(VectorIndexError::InvalidDimension { dimension });
        }
        Ok(Self {
            dimension,
            documents: Vec::new(),
        })
    }

    /// Returns the index vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the number of stored document records, including deleted records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Returns whether the index has no stored document records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn upsert(&mut self, document: VectorDocument) -> Result<(), VectorIndexError> {
        validate_dimension(self.dimension, document.vector().dimension())?;

        if let Some(existing) = self
            .documents
            .iter_mut()
            .find(|stored| stored.document.doc_id() == document.doc_id())
        {
            existing.document = document;
            existing.deleted = false;
            return Ok(());
        }

        self.documents.push(StoredVectorDocument {
            document,
            deleted: false,
        });
        Ok(())
    }

    fn mark_deleted(&mut self, doc_id: &str) -> Result<(), VectorIndexError> {
        if let Some(existing) = self
            .documents
            .iter_mut()
            .find(|stored| stored.document.doc_id() == doc_id)
        {
            existing.deleted = true;
        }
        Ok(())
    }

    fn search(&self, request: &VectorSearchRequest) -> Result<Vec<VectorHit>, VectorIndexError> {
        validate_dimension(self.dimension, request.query().dimension())?;

        let mut hits = self
            .documents
            .iter()
            .filter(|stored| !stored.deleted)
            .map(|stored| {
                VectorHit::new(
                    stored.document.doc_id(),
                    score(
                        request.metric(),
                        request.query().values(),
                        stored.document.vector().values(),
                    ),
                )
            })
            .collect::<Vec<_>>();

        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.doc_id.cmp(&right.doc_id))
        });
        hits.truncate(request.top_k());
        Ok(hits)
    }
}

impl fmt::Debug for InMemoryVectorIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InMemoryVectorIndex")
            .field("vector_dimension", &self.dimension)
            .field("document_count", &self.documents.len())
            .field(
                "deleted_count",
                &self
                    .documents
                    .iter()
                    .filter(|stored| stored.deleted)
                    .count(),
            )
            .finish()
    }
}

#[derive(Clone, PartialEq)]
struct StoredVectorDocument {
    document: VectorDocument,
    deleted: bool,
}

/// Errors returned by vector index implementations.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum VectorIndexError {
    /// Vector dimension must be greater than zero.
    #[error("vector dimension must be greater than zero, got {dimension}")]
    InvalidDimension {
        /// Invalid dimension value.
        dimension: usize,
    },
    /// A vector dimension did not match the index dimension.
    #[error("vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected vector dimension.
        expected: usize,
        /// Actual vector dimension.
        actual: usize,
    },
    /// Search requests must ask for at least one result.
    #[error("top_k must be greater than zero, got {top_k}")]
    InvalidTopK {
        /// Invalid top-k value.
        top_k: usize,
    },
}

fn validate_dimension(expected: usize, actual: usize) -> Result<(), VectorIndexError> {
    if expected == actual {
        Ok(())
    } else {
        Err(VectorIndexError::DimensionMismatch { expected, actual })
    }
}

fn score(metric: SimilarityMetric, query: &[f32], candidate: &[f32]) -> f32 {
    match metric {
        SimilarityMetric::Cosine => cosine(query, candidate),
        SimilarityMetric::Dot => dot(query, candidate),
    }
}

fn cosine(query: &[f32], candidate: &[f32]) -> f32 {
    let denominator = norm(query) * norm(candidate);
    if denominator.abs() < f32::EPSILON {
        0.0
    } else {
        dot(query, candidate) / denominator
    }
}

fn dot(query: &[f32], candidate: &[f32]) -> f32 {
    query
        .iter()
        .zip(candidate.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}
