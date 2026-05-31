pub fn crate_name() -> &'static str {
    "embedder"
}

use std::fmt;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub trait Embedder {
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn embed_batch(
        &self,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddingBudget {
    max_inputs: usize,
    max_text_bytes: usize,
}

impl EmbeddingBudget {
    pub fn new(max_inputs: usize, max_text_bytes: usize) -> Self {
        Self {
            max_inputs,
            max_text_bytes,
        }
    }

    pub fn max_inputs(self) -> usize {
        self.max_inputs
    }

    pub fn max_text_bytes(self) -> usize {
        self.max_text_bytes
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingInput {
    id: String,
    text: String,
}

impl EmbeddingInput {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for EmbeddingInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingInput")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct EmbeddingVector {
    id: String,
    model_id: String,
    values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn new(
        id: impl Into<String>,
        model_id: impl Into<String>,
        values: Vec<f32>,
    ) -> Result<Self, EmbeddingError> {
        if values.is_empty() {
            return Err(EmbeddingError::InvalidDimension);
        }

        Ok(Self {
            id: id.into(),
            model_id: model_id.into(),
            values,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

impl fmt::Debug for EmbeddingVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingVector")
            .field("id", &self.id)
            .field("model_id", &self.model_id)
            .field("dimension", &self.values.len())
            .finish()
    }
}

/// Deterministic local embedder for tests and interface wiring only.
///
/// It is a lexical hash vectorizer, not a licensed model and not a semantic
/// quality claim.
#[derive(Clone, PartialEq)]
pub struct DeterministicTestEmbedder {
    model_id: String,
    dimension: usize,
}

impl DeterministicTestEmbedder {
    pub fn new(model_id: impl Into<String>, dimension: usize) -> Result<Self, EmbeddingError> {
        if dimension == 0 {
            return Err(EmbeddingError::InvalidDimension);
        }

        Ok(Self {
            model_id: model_id.into(),
            dimension,
        })
    }
}

impl fmt::Debug for DeterministicTestEmbedder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeterministicTestEmbedder")
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .finish()
    }
}

impl Embedder for DeterministicTestEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_batch(
        &self,
        inputs: &[EmbeddingInput],
        budget: EmbeddingBudget,
    ) -> Result<Vec<EmbeddingVector>, EmbeddingError> {
        if inputs.len() > budget.max_inputs() {
            return Err(EmbeddingError::BudgetExceeded {
                limit: budget.max_inputs(),
                actual: inputs.len(),
            });
        }

        inputs
            .iter()
            .map(|input| {
                if input.text().len() > budget.max_text_bytes() {
                    return Err(EmbeddingError::TextBudgetExceeded {
                        limit: budget.max_text_bytes(),
                        actual: input.text().len(),
                    });
                }

                EmbeddingVector::new(
                    input.id(),
                    self.model_id(),
                    deterministic_values(input.text(), self.dimension),
                )
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingError {
    InvalidDimension,
    BudgetExceeded { limit: usize, actual: usize },
    TextBudgetExceeded { limit: usize, actual: usize },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimension => formatter.write_str("embedding dimension must be positive"),
            Self::BudgetExceeded { limit, actual } => {
                write!(
                    formatter,
                    "embedding batch limit {limit} exceeded by {actual}"
                )
            }
            Self::TextBudgetExceeded { limit, actual } => {
                write!(
                    formatter,
                    "embedding text byte limit {limit} exceeded by {actual}"
                )
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

fn deterministic_values(text: &str, dimension: usize) -> Vec<f32> {
    let mut values = vec![0.0; dimension];

    for token in text.split_whitespace() {
        let normalized = token.to_ascii_lowercase();
        let hash = stable_hash(normalized.as_bytes());
        let index = hash as usize % dimension;
        values[index] += 1.0;
    }

    let magnitude = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut values {
            *value /= magnitude;
        }
    }

    values
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
