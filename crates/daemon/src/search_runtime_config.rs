use embedder::ResidentEmbeddingClient;

/// The complete configuration required by the resident search runtime.
///
/// The composition root projects this value from process-wide run options so
/// the query service cannot depend on unrelated import, OCR, or lifecycle
/// configuration.
#[derive(Clone)]
pub(crate) struct SearchRuntimeConfig {
    pub(crate) resident_embedding: Option<ResidentEmbeddingClient>,
    pub(crate) embedding_model_id: Option<String>,
    pub(crate) embedding_dimension: Option<usize>,
    pub(crate) embedding_timeout_ms: u64,
}

impl SearchRuntimeConfig {
    pub(crate) fn new(
        resident_embedding: Option<ResidentEmbeddingClient>,
        embedding_model_id: Option<String>,
        embedding_dimension: Option<usize>,
        embedding_timeout_ms: u64,
    ) -> Self {
        Self {
            resident_embedding,
            embedding_model_id,
            embedding_dimension,
            embedding_timeout_ms,
        }
    }
}
