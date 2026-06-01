use embedder::{DeterministicTestEmbedder, Embedder, EmbeddingBudget, EmbeddingInput};
use index_vector::{
    inspect_persistent_vector_snapshot, InMemoryVectorIndex, PersistentVectorIndex,
    PersistentVectorSnapshotState, QueryVector, VectorDocument, VectorIndex,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[test]
fn persistent_vector_index_reopens_snapshot_and_preserves_tombstones_without_path_leakage() {
    let private_dir = temp_dir("private-vector-index");
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

    {
        let index = PersistentVectorIndex::open(&private_dir, 16).unwrap();
        index
            .upsert(vec![
                VectorDocument::new("vec_java", "doc_java", vectors[0].values().to_vec()).unwrap(),
                VectorDocument::new("vec_data", "doc_data", vectors[1].values().to_vec()).unwrap(),
            ])
            .unwrap();
        index.mark_deleted(&["vec_java"]).unwrap();
        let debug = format!("{index:?}");
        assert!(!debug.contains(path_str(&private_dir)));
        assert!(!debug.contains("0."));
    }

    {
        let reopened = PersistentVectorIndex::open(&private_dir, 16).unwrap();
        let hits = reopened
            .knn(QueryVector::new(query.values().to_vec()).unwrap(), 2)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id(), "doc_data");
        let snapshot = reopened.snapshot().unwrap();
        assert_eq!(snapshot.vector_count(), 2);
        assert_eq!(snapshot.deleted_count(), 1);
        assert_eq!(snapshot.dimension(), 16);
        let inspection = inspect_persistent_vector_snapshot(&private_dir);
        assert_eq!(inspection.state(), PersistentVectorSnapshotState::Ready);
        assert_eq!(
            inspection
                .snapshot()
                .map(|snapshot| snapshot.vector_count()),
            Some(2)
        );
    }

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_rejects_corrupt_snapshot_without_path_leakage() {
    let private_dir = temp_dir("private-corrupt-vector-index");
    fs::write(
        private_dir.join("vector.snapshot"),
        "resume-ir-vector-index-v1\tdimension\t16\nV\tbad\tbad\tffffffff\n",
    )
    .unwrap();

    let error = PersistentVectorIndex::open(&private_dir, 16).unwrap_err();
    assert_eq!(
        inspect_persistent_vector_snapshot(&private_dir).state(),
        PersistentVectorSnapshotState::Corrupt
    );
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains(path_str(&private_dir)));
    assert!(!error.to_string().contains(path_str(&private_dir)));

    remove_dir(&private_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-vector-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
