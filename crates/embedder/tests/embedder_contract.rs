//! Embedder contract tests for the dependency-light S11 skeleton.

use embedder::{
    Embedder, EmbedderError, EmbeddingInput, EmbeddingRequest, FakeEmbedder, FakeEmbedderConfig,
};
use std::error::Error;

#[test]
fn fake_embedder_returns_deterministic_vectors_and_redacts_text() -> Result<(), Box<dyn Error>> {
    let request = EmbeddingRequest::new(vec![EmbeddingInput::new(
        "synthetic rust backend resume text",
    )]);
    let embedder = FakeEmbedder::new(FakeEmbedderConfig::new(6))?;

    let first = embedder.embed(&request)?;
    let second = embedder.embed(&request)?;

    assert_eq!(first.vectors(), second.vectors());
    assert_eq!(first.vectors().len(), 1);
    assert_eq!(first.vectors()[0].dimension(), 6);
    assert!(first.vectors()[0]
        .values()
        .iter()
        .any(|value| *value != 0.0));

    let debug = format!("{request:?} {first:?} {embedder:?}");
    assert!(debug.contains("input_count"));
    assert!(debug.contains("vector_count"));
    assert!(!debug.contains("synthetic rust backend resume text"));

    Ok(())
}

#[test]
fn fake_embedder_validates_dimension_and_batch_shape() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        FakeEmbedder::new(FakeEmbedderConfig::new(0)),
        Err(EmbedderError::InvalidDimension { dimension: 0 })
    ));

    let embedder = FakeEmbedder::new(FakeEmbedderConfig::new(3))?;
    assert!(matches!(
        embedder.embed(&EmbeddingRequest::new(Vec::new())),
        Err(EmbedderError::EmptyBatch)
    ));

    Ok(())
}
