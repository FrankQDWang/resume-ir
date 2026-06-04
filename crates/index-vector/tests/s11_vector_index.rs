use embedder::{DeterministicTestEmbedder, Embedder, EmbeddingBudget, EmbeddingInput};
use index_vector::{
    inspect_persistent_vector_snapshot, InMemoryVectorIndex, PersistentVectorIndex,
    PersistentVectorSnapshotState, QueryVector, VectorDocument, VectorIndex, VectorSearchBackend,
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
fn persistent_vector_index_filters_knn_by_model_scope_after_reopen() {
    let private_dir = temp_dir("private-model-scoped-vector-index");

    {
        let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        index
            .upsert(vec![
                VectorDocument::new_for_model(
                    "model-a",
                    "model-a:vec_legacy",
                    "doc_legacy_model",
                    vec![1.0, 0.0, 0.0, 0.0],
                )
                .unwrap(),
                VectorDocument::new_for_model(
                    "model-b",
                    "model-b:vec_current",
                    "doc_current_model",
                    vec![0.0, 1.0, 0.0, 0.0],
                )
                .unwrap(),
            ])
            .unwrap();
    }

    {
        let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        let unscoped = reopened
            .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 1)
            .unwrap();
        assert_eq!(unscoped[0].doc_id(), "doc_legacy_model");

        let scoped = reopened
            .knn_for_model(
                QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
                1,
                "model-b",
            )
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].doc_id(), "doc_current_model");
        assert_eq!(scoped[0].vector_id(), "model-b:vec_current");
    }

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_uses_hnsw_ann_backend_after_reopen_and_keeps_model_scope() {
    let private_dir = temp_dir("private-hnsw-vector-index");

    {
        let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        let mut documents = (0..48)
            .map(|number| {
                VectorDocument::new_for_model(
                    "model-a",
                    format!("model-a:vec_irrelevant_{number:03}"),
                    format!("doc_irrelevant_{number:03}"),
                    vec![1.0, 0.0, 0.0, 0.0],
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        documents.push(
            VectorDocument::new_for_model(
                "model-b",
                "model-b:vec_target",
                "doc_target_model_b",
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap(),
        );
        index.upsert(documents).unwrap();
    }

    {
        let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap().search_backend(),
            VectorSearchBackend::HnswAnn
        );

        let scoped = reopened
            .knn_for_model(
                QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
                1,
                "model-b",
            )
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].doc_id(), "doc_target_model_b");
        assert_eq!(scoped[0].vector_id(), "model-b:vec_target");
    }

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_rebuilds_hnsw_after_upsert_and_tombstone() {
    let private_dir = temp_dir("private-hnsw-stale-node-vector-index");
    let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    index
        .upsert(vec![
            VectorDocument::new("vec_moving", "doc_moving", vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
            VectorDocument::new("vec_stable", "doc_stable", vec![0.0, 1.0, 0.0, 0.0]).unwrap(),
        ])
        .unwrap();

    let first_hits = index
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 1)
        .unwrap();
    assert_eq!(first_hits[0].doc_id(), "doc_moving");
    assert!(first_hits[0].score() > 0.99);

    index
        .upsert(vec![VectorDocument::new(
            "vec_moving",
            "doc_moving",
            vec![0.0, 1.0, 0.0, 0.0],
        )
        .unwrap()])
        .unwrap();
    let after_update = index
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 2)
        .unwrap();
    assert!(after_update.iter().all(|hit| hit.score() < 0.01));

    index.mark_deleted(&["vec_moving"]).unwrap();
    let after_delete = index
        .knn(QueryVector::new(vec![0.0, 1.0, 0.0, 0.0]).unwrap(), 2)
        .unwrap();
    assert_eq!(after_delete.len(), 1);
    assert_eq!(after_delete[0].doc_id(), "doc_stable");

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_filters_legacy_v1_snapshot_by_vector_id_model_prefix() {
    let private_dir = temp_dir("private-legacy-model-scoped-vector-index");
    fs::write(
        private_dir.join("vector.snapshot"),
        concat!(
            "resume-ir-vector-index-v1\tdimension\t4\n",
            "V\tmodel-a%3Avec_legacy\tdoc_legacy_model\t3f800000,00000000,00000000,00000000\n",
            "V\tmodel-b%3Avec_current\tdoc_current_model\t00000000,3f800000,00000000,00000000\n",
        ),
    )
    .unwrap();

    let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let scoped = index
        .knn_for_model(
            QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
            1,
            "model-b",
        )
        .unwrap();

    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].doc_id(), "doc_current_model");
    assert_eq!(index.snapshot().unwrap().vector_count(), 2);

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
