use embedder::{DeterministicTestEmbedder, Embedder, EmbeddingBudget, EmbeddingInput};
use index_vector::{InMemoryVectorIndex, QueryVector, VectorDocument, VectorIndex};

#[test]
fn exposes_index_vector_crate_identity() {
    assert_eq!(index_vector::crate_name(), "index-vector");
}

#[test]
fn in_memory_vector_index_searches_marks_deleted_and_snapshots_without_vector_leakage() {
    let embedder = DeterministicTestEmbedder::new("test-lexical-hash", 16).unwrap();
    let vectors = embedder
        .embed_batch(
            &[
                EmbeddingInput::new("doc_java", "Java payment backend"),
                EmbeddingInput::new("doc_data", "analytics warehouse governance"),
            ],
            EmbeddingBudget::new(2, 128),
        )
        .unwrap();
    let query = embedder
        .embed_batch(
            &[EmbeddingInput::new("query", "Java backend")],
            EmbeddingBudget::new(1, 64),
        )
        .unwrap()
        .remove(0);
    let index = InMemoryVectorIndex::new(16);

    index
        .upsert(vec![
            VectorDocument::new("vec_java", "doc_java", vectors[0].values().to_vec()).unwrap(),
            VectorDocument::new("vec_data", "doc_data", vectors[1].values().to_vec()).unwrap(),
        ])
        .unwrap();

    let hits = index
        .knn(QueryVector::new(query.values().to_vec()).unwrap(), 2)
        .unwrap();
    assert_eq!(hits[0].doc_id(), "doc_java");
    assert!(hits[0].score() >= hits[1].score());
    assert!(!format!("{:?}", hits[0]).contains("Java"));
    assert!(!format!("{:?}", query).contains("0."));

    index.mark_deleted(&["vec_java"]).unwrap();
    let after_delete = index
        .knn(QueryVector::new(query.values().to_vec()).unwrap(), 2)
        .unwrap();
    assert_eq!(after_delete[0].doc_id(), "doc_data");

    let snapshot = index.snapshot().unwrap();
    assert_eq!(snapshot.vector_count(), 2);
    assert_eq!(snapshot.deleted_count(), 1);
    assert_eq!(snapshot.dimension(), 16);
}
