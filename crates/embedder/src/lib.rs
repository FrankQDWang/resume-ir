//! Embedding interfaces for the semantic retrieval skeleton.

use std::fmt;
use thiserror::Error;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One local text input to embed.
#[derive(Clone, Eq, PartialEq)]
pub struct EmbeddingInput {
    text: String,
}

impl EmbeddingInput {
    /// Creates an embedding input from local text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// Returns the local text for embedder implementations.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for EmbeddingInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingInput")
            .field("text", &"[redacted embedding input]")
            .finish()
    }
}

/// Batch request for an embedder.
#[derive(Clone, Eq, PartialEq)]
pub struct EmbeddingRequest {
    inputs: Vec<EmbeddingInput>,
}

impl EmbeddingRequest {
    /// Creates a batch request.
    #[must_use]
    pub fn new(inputs: Vec<EmbeddingInput>) -> Self {
        Self { inputs }
    }

    /// Returns the request inputs.
    #[must_use]
    pub fn inputs(&self) -> &[EmbeddingInput] {
        &self.inputs
    }

    /// Returns whether the request has no inputs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }
}

impl fmt::Debug for EmbeddingRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingRequest")
            .field("input_count", &self.inputs.len())
            .finish()
    }
}

/// Dense embedding vector.
#[derive(Clone, PartialEq)]
pub struct EmbeddingVector {
    values: Vec<f32>,
}

impl EmbeddingVector {
    /// Creates a vector after validating dimension and finite values.
    pub fn new(values: Vec<f32>) -> Result<Self, EmbedderError> {
        if values.is_empty() {
            return Err(EmbedderError::InvalidDimension { dimension: 0 });
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(EmbedderError::NonFiniteValue);
        }
        Ok(Self { values })
    }

    /// Returns the vector values for similarity implementations.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Returns the vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.values.len()
    }
}

impl fmt::Debug for EmbeddingVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingVector")
            .field("dimension", &self.dimension())
            .field("values", &"[redacted embedding vector]")
            .finish()
    }
}

/// Embedder response containing one vector per input.
#[derive(Clone, PartialEq)]
pub struct EmbeddingResponse {
    vectors: Vec<EmbeddingVector>,
}

impl EmbeddingResponse {
    /// Creates an embedder response.
    #[must_use]
    pub fn new(vectors: Vec<EmbeddingVector>) -> Self {
        Self { vectors }
    }

    /// Returns embedded vectors in request order.
    #[must_use]
    pub fn vectors(&self) -> &[EmbeddingVector] {
        &self.vectors
    }
}

impl fmt::Debug for EmbeddingResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingResponse")
            .field("vector_count", &self.vectors.len())
            .field(
                "dimensions",
                &self
                    .vectors
                    .iter()
                    .map(EmbeddingVector::dimension)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Synchronous embedding provider interface.
pub trait Embedder {
    /// Embeds a batch of local text inputs.
    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse, EmbedderError>;
}

/// Configuration for the deterministic fake embedder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FakeEmbedderConfig {
    dimension: usize,
}

impl FakeEmbedderConfig {
    /// Creates fake embedder configuration.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Returns the configured vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Deterministic fake embedder for tests and interface wiring.
#[derive(Clone, Eq, PartialEq)]
pub struct FakeEmbedder {
    config: FakeEmbedderConfig,
}

impl FakeEmbedder {
    /// Creates a fake embedder after validating the configured dimension.
    pub fn new(config: FakeEmbedderConfig) -> Result<Self, EmbedderError> {
        if config.dimension == 0 {
            return Err(EmbedderError::InvalidDimension {
                dimension: config.dimension,
            });
        }
        Ok(Self { config })
    }

    /// Returns the fake embedder configuration.
    #[must_use]
    pub fn config(&self) -> FakeEmbedderConfig {
        self.config
    }
}

impl Embedder for FakeEmbedder {
    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse, EmbedderError> {
        if request.is_empty() {
            return Err(EmbedderError::EmptyBatch);
        }

        let mut vectors = Vec::with_capacity(request.inputs().len());
        for input in request.inputs() {
            vectors.push(EmbeddingVector::new(fake_values(
                input.text(),
                self.config.dimension,
            ))?);
        }

        Ok(EmbeddingResponse::new(vectors))
    }
}

impl fmt::Debug for FakeEmbedder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeEmbedder")
            .field("dimension", &self.config.dimension)
            .finish()
    }
}

/// Errors returned by embedder implementations and vector validation.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum EmbedderError {
    /// Vector dimension must be greater than zero.
    #[error("embedding dimension must be greater than zero, got {dimension}")]
    InvalidDimension {
        /// Invalid dimension value.
        dimension: usize,
    },
    /// Batch requests must contain at least one input.
    #[error("embedding request must contain at least one input")]
    EmptyBatch,
    /// Vector values must be finite.
    #[error("embedding vector contains a non-finite value")]
    NonFiniteValue,
}

fn fake_values(text: &str, dimension: usize) -> Vec<f32> {
    let mut values = Vec::with_capacity(dimension);
    for index in 0..dimension {
        let hash = fake_hash(text.as_bytes(), index);
        let bucket = (hash % 2001) as f32;
        let mut value = (bucket / 1000.0) - 1.0;
        if value.abs() < f32::EPSILON {
            value = 0.001;
        }
        values.push(value);
    }

    if values.iter().all(|value| value.abs() < f32::EPSILON) {
        values[0] = 0.001;
    }

    values
}

fn fake_hash(bytes: &[u8], index: usize) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in index.to_le_bytes().iter().chain(bytes.iter()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
