use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub type EmbeddingResult<T> = Result<T, EmbeddingError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingError {
    message: String,
}

impl EmbeddingError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Embedding {
    pub model_id: String,
    pub values: Vec<f32>,
}

pub trait Embedder {
    fn model_id(&self) -> &str;
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeEmbedder {
    dim: usize,
    model_id: String,
}

impl FakeEmbedder {
    #[must_use]
    pub fn new(dim: usize) -> Self {
        Self {
            dim: dim.max(1),
            model_id: "fake-local-hash-v1".to_owned(),
        }
    }
}

impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        let mut values = vec![0.0; self.dim];
        for token in text.split_whitespace() {
            let normalized = token.to_ascii_lowercase();
            let mut hasher = DefaultHasher::new();
            normalized.hash(&mut hasher);
            let bucket = hasher.finish() as usize % self.dim;
            values[bucket] += 1.0;
        }
        normalize(&mut values);
        Ok(Embedding {
            model_id: self.model_id.clone(),
            values,
        })
    }
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in values {
        *value /= norm;
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "embedder"
}
