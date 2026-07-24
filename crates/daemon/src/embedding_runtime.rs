use std::sync::Arc;

use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingClient, ResidentEmbeddingOwner, ResidentEmbeddingSpec,
};
use import_pipeline::{
    ImportResourcePolicy, SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationVectorization, SearchPublicationVectorizer,
};

use crate::daemon_error::{DaemonError, Result};
use crate::run_options::{usage, RunOptions};

pub(crate) fn start(options: &mut RunOptions) -> Result<Option<ResidentEmbeddingOwner>> {
    if options.embedding_command.is_none() {
        return Ok(None);
    }
    let command = options
        .embedding_command
        .clone()
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let command = crate::runtime_pack::validated_embedding_command(&command)
        .map_err(|_| {
            DaemonError::configuration_invalid(
                "embedding runtime executable attestation failed before spawn",
            )
        })?
        .into_path();
    let model_id = options
        .embedding_model_id
        .as_deref()
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let dimension = options
        .embedding_dimension
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let command =
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(DaemonError::embedding)?
            .with_timeout_ms(options.embedding_timeout_ms)
            .map_err(DaemonError::embedding)?;
    let inference_threads = ImportResourcePolicy::detect().parse_workers.get();
    let owner = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::new(command)
            .with_intra_threads(inference_threads)
            .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let client = owner.client();
    options.search_vectorization =
        SearchPublicationVectorization::enabled(Arc::new(ResidentPublicationVectorizer {
            client: client.clone(),
            timeout_ms: options.embedding_timeout_ms,
        }));
    options.resident_embedding = Some(client);
    Ok(Some(owner))
}

struct ResidentPublicationVectorizer {
    client: ResidentEmbeddingClient,
    timeout_ms: u64,
}

impl SearchPublicationVectorizer for ResidentPublicationVectorizer {
    fn model_id(&self) -> &str {
        self.client.model_id()
    }

    fn dimension(&self) -> usize {
        self.client.dimension()
    }

    fn max_batch_inputs(&self) -> usize {
        embedding_protocol::MAX_INPUTS
    }

    fn max_text_bytes(&self) -> usize {
        embedding_protocol::MAX_TEXT_BYTES
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure>
    {
        let resident_inputs = inputs
            .iter()
            .map(|input| EmbeddingInput::new(input.id(), input.text()))
            .collect::<Vec<_>>();
        self.client
            .embed_batch_with_cancel(
                EmbeddingPriority::Background,
                &resident_inputs,
                EmbeddingBudget::new(resident_inputs.len(), embedding_protocol::MAX_TEXT_BYTES),
                self.timeout_ms,
                is_cancelled,
            )
            .map(|outputs| {
                outputs
                    .into_iter()
                    .map(|output| {
                        SearchPublicationEmbeddingOutput::new(
                            output.id(),
                            output.model_id(),
                            output.values().to_vec(),
                        )
                    })
                    .collect()
            })
            .map_err(|error| match error {
                EmbeddingError::Cancelled => SearchPublicationEmbeddingFailure::Cancelled,
                EmbeddingError::InvalidDimension
                | EmbeddingError::InvalidRequest
                | EmbeddingError::BudgetExceeded { .. }
                | EmbeddingError::TextBudgetExceeded { .. } => {
                    SearchPublicationEmbeddingFailure::InvalidOutput
                }
                EmbeddingError::WorkerUnavailable
                | EmbeddingError::EngineFailed
                | EmbeddingError::Overloaded
                | EmbeddingError::Timeout => SearchPublicationEmbeddingFailure::RuntimeUnavailable,
            })
    }
}
