//! Vector index contract tests for the dependency-light S11 skeleton.

use embedder::EmbeddingVector;
use index_vector::{
    InMemoryVectorIndex, SimilarityMetric, VectorDocument, VectorIndex, VectorIndexError,
    VectorSearchRequest,
};
use std::error::Error;

#[test]
fn in_memory_index_filters_deleted_docs_and_orders_cosine_results() -> Result<(), Box<dyn Error>> {
    let mut index = InMemoryVectorIndex::new(2)?;
    index.upsert(VectorDocument::new(
        "doc-b",
        EmbeddingVector::new(vec![0.5, 0.5])?,
    ))?;
    index.upsert(VectorDocument::new(
        "doc-a",
        EmbeddingVector::new(vec![1.0, 0.0])?,
    ))?;
    index.upsert(VectorDocument::new(
        "doc-c",
        EmbeddingVector::new(vec![0.0, 1.0])?,
    ))?;
    index.mark_deleted("doc-c")?;

    let hits = index.search(&VectorSearchRequest::new(
        EmbeddingVector::new(vec![1.0, 0.0])?,
        SimilarityMetric::Cosine,
        10,
    )?)?;

    assert_eq!(
        hits.iter().map(|hit| hit.doc_id()).collect::<Vec<_>>(),
        ["doc-a", "doc-b"]
    );
    assert!(hits[0].score() > hits[1].score());

    Ok(())
}

#[test]
fn in_memory_index_checks_dot_similarity_ties_and_dimensions() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        InMemoryVectorIndex::new(0),
        Err(VectorIndexError::InvalidDimension { dimension: 0 })
    ));

    let mut index = InMemoryVectorIndex::new(3)?;
    index.upsert(VectorDocument::new(
        "doc-b",
        EmbeddingVector::new(vec![1.0, 0.0, 0.0])?,
    ))?;
    index.upsert(VectorDocument::new(
        "doc-a",
        EmbeddingVector::new(vec![1.0, 0.0, 0.0])?,
    ))?;
    assert!(matches!(
        index.upsert(VectorDocument::new(
            "bad-dimension",
            EmbeddingVector::new(vec![1.0, 0.0])?,
        )),
        Err(VectorIndexError::DimensionMismatch {
            expected: 3,
            actual: 2
        })
    ));

    let hits = index.search(&VectorSearchRequest::new(
        EmbeddingVector::new(vec![1.0, 0.0, 0.0])?,
        SimilarityMetric::Dot,
        1,
    )?)?;

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id(), "doc-a");
    assert!((hits[0].score() - 1.0).abs() < f32::EPSILON);

    Ok(())
}

#[test]
fn debug_output_redacts_vector_payloads() -> Result<(), Box<dyn Error>> {
    let mut index = InMemoryVectorIndex::new(2)?;
    index.upsert(VectorDocument::new(
        "doc-safe",
        EmbeddingVector::new(vec![0.12345, 0.54321])?,
    ))?;
    let request = VectorSearchRequest::new(
        EmbeddingVector::new(vec![0.12345, 0.54321])?,
        SimilarityMetric::Cosine,
        5,
    )?;
    let hits = index.search(&request)?;

    let debug = format!("{index:?} {request:?} {hits:?}");
    assert!(debug.contains("vector_dimension"));
    assert!(!debug.contains("0.12345"));
    assert!(!debug.contains("0.54321"));

    Ok(())
}
