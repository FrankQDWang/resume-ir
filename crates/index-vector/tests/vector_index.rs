use embedder::{Embedder, FakeEmbedder};
use index_vector::{InMemoryVectorIndex, VectorDocument, VectorIndex};

#[test]
fn in_memory_vector_index_returns_nearest_vectors() {
    let embedder = FakeEmbedder::new(8);
    let query = embedder.embed("Java payment backend").expect("query");
    let java = embedder.embed("Java payment service").expect("java");
    let design = embedder.embed("product design research").expect("design");
    let mut index = InMemoryVectorIndex::default();

    index.upsert(VectorDocument {
        doc_id: "doc_java".to_owned(),
        vector: java.values,
    });
    index.upsert(VectorDocument {
        doc_id: "doc_design".to_owned(),
        vector: design.values,
    });

    let hits = index.search(&query.values, 1);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_java");
    assert!(hits[0].score > 0.0);
}

#[test]
fn upsert_replaces_existing_vector_for_document() {
    let mut index = InMemoryVectorIndex::default();

    index.upsert(VectorDocument {
        doc_id: "doc_java".to_owned(),
        vector: vec![1.0, 0.0],
    });
    index.upsert(VectorDocument {
        doc_id: "doc_java".to_owned(),
        vector: vec![0.0, 1.0],
    });

    let hits = index.search(&[0.0, 1.0], 10);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, "doc_java");
    assert!(hits[0].score > 0.99);
}
