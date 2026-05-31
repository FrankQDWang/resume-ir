use embedder::{DeterministicTestEmbedder, Embedder, EmbeddingBudget, EmbeddingInput};

#[test]
fn exposes_embedder_crate_identity() {
    assert_eq!(embedder::crate_name(), "embedder");
}

#[test]
fn deterministic_test_embedder_is_stable_and_budgeted_without_text_leakage() {
    let embedder = DeterministicTestEmbedder::new("test-lexical-hash", 8).unwrap();
    let inputs = [
        EmbeddingInput::new("doc_java", "Java Spring Cloud platform"),
        EmbeddingInput::new("doc_rust", "Rust search index"),
    ];

    let vectors = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(2, 128))
        .unwrap();
    let repeated = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(2, 128))
        .unwrap();

    assert_eq!(vectors, repeated);
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].id(), "doc_java");
    assert_eq!(vectors[0].model_id(), "test-lexical-hash");
    assert_eq!(vectors[0].values().len(), 8);
    assert!(vectors[0].values().iter().any(|value| *value != 0.0));
    assert!(!format!("{:?}", inputs[0]).contains("Java"));
    assert!(!format!("{:?}", vectors[0]).contains("0."));

    let error = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(1, 128))
        .unwrap_err();
    assert!(!format!("{error:?}").contains("Java"));
}
