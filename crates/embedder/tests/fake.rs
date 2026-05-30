use embedder::{Embedder, FakeEmbedder};

#[test]
fn fake_embedder_is_deterministic_and_model_free() {
    let embedder = FakeEmbedder::new(8);

    let first = embedder.embed("Java payment backend").expect("embed first");
    let second = embedder
        .embed("Java payment backend")
        .expect("embed second");

    assert_eq!(embedder.model_id(), "fake-local-hash-v1");
    assert_eq!(first.model_id, "fake-local-hash-v1");
    assert_eq!(first.values.len(), 8);
    assert_eq!(first, second);
    assert!(first.values.iter().any(|value| *value != 0.0));
}

#[test]
fn fake_embedder_produces_different_vectors_for_different_text() {
    let embedder = FakeEmbedder::new(8);

    let java = embedder.embed("Java payment backend").expect("embed java");
    let design = embedder
        .embed("product design research")
        .expect("embed design");

    assert_ne!(java.values, design.values);
}
