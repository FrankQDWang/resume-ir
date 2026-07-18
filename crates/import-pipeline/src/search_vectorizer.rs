use std::fmt;
use std::sync::Arc;

/// Produces bounded document embeddings for one atomic search publication.
///
/// Implementations must return exactly one output for every input, preserve
/// each opaque input ID, and bind every output to `model_id()` and
/// `dimension()`. Implementations may perform local inference, but must not
/// persist vectors or mutate metadata; the publication transaction owns those
/// effects.
pub trait SearchPublicationVectorizer: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn max_batch_inputs(&self) -> usize;
    fn max_text_bytes(&self) -> usize;
    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure>;
}

#[derive(Clone)]
pub struct SearchPublicationEmbeddingInput {
    id: String,
    text: String,
}

impl SearchPublicationEmbeddingInput {
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

impl fmt::Debug for SearchPublicationEmbeddingInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchPublicationEmbeddingInput")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct SearchPublicationEmbeddingOutput {
    id: String,
    model_id: String,
    values: Vec<f32>,
}

impl SearchPublicationEmbeddingOutput {
    pub fn new(id: impl Into<String>, model_id: impl Into<String>, values: Vec<f32>) -> Self {
        Self {
            id: id.into(),
            model_id: model_id.into(),
            values,
        }
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

impl fmt::Debug for SearchPublicationEmbeddingOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchPublicationEmbeddingOutput")
            .field("id", &self.id)
            .field("model_id", &self.model_id)
            .field("dimension", &self.values.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationEmbeddingFailure {
    Cancelled,
    RuntimeUnavailable,
    InvalidOutput,
}

#[derive(Clone, Default)]
pub enum SearchPublicationVectorization {
    #[default]
    Disabled,
    Enabled(Arc<dyn SearchPublicationVectorizer>),
}

impl SearchPublicationVectorization {
    pub fn enabled(vectorizer: Arc<dyn SearchPublicationVectorizer>) -> Self {
        Self::Enabled(vectorizer)
    }

    pub(crate) fn vectorizer(&self) -> Option<&dyn SearchPublicationVectorizer> {
        match self {
            Self::Disabled => None,
            Self::Enabled(vectorizer) => Some(vectorizer.as_ref()),
        }
    }
}

impl fmt::Debug for SearchPublicationVectorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Disabled"),
            Self::Enabled(vectorizer) => formatter
                .debug_struct("Enabled")
                .field("model_id", &vectorizer.model_id())
                .field("dimension", &vectorizer.dimension())
                .field("max_batch_inputs", &vectorizer.max_batch_inputs())
                .field("max_text_bytes", &vectorizer.max_text_bytes())
                .finish(),
        }
    }
}
