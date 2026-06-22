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
    assert_eq!(snapshot.document_count(), 1);
    assert_eq!(snapshot.dimension(), 16);
}

#[test]
fn vector_snapshot_counts_unique_active_documents_not_section_vectors() {
    let index = InMemoryVectorIndex::new(4);

    index
        .upsert(vec![
            VectorDocument::new("doc_a:main", "doc_a", vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
            VectorDocument::new("doc_a:section:0", "doc_a", vec![0.9, 0.1, 0.0, 0.0]).unwrap(),
            VectorDocument::new("doc_b:main", "doc_b", vec![0.0, 1.0, 0.0, 0.0]).unwrap(),
            VectorDocument::new("doc_deleted:main", "doc_deleted", vec![0.0, 0.0, 1.0, 0.0])
                .unwrap(),
        ])
        .unwrap();
    index.mark_deleted(&["doc_deleted:main"]).unwrap();

    let snapshot = index.snapshot().unwrap();

    assert_eq!(snapshot.vector_count(), 4);
    assert_eq!(snapshot.deleted_count(), 1);
    assert_eq!(snapshot.document_count(), 2);
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
fn persistent_vector_index_encrypts_snapshot_payload_at_rest() {
    let private_dir = temp_dir("private-encrypted-vector-index");
    let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    index
        .upsert(vec![VectorDocument::new_for_model(
            "model-private",
            "model-private:vec_secret",
            "doc_secret",
            vec![1.0, 0.5, 0.25, 0.125],
        )
        .unwrap()])
        .unwrap();

    let snapshot_bytes = fs::read(private_dir.join("vector.snapshot")).unwrap();
    let snapshot_text = String::from_utf8_lossy(&snapshot_bytes);
    assert!(snapshot_text.starts_with("resume-ir-vector-index-encrypted-v1\n"));
    assert!(!snapshot_text.contains("model-private"));
    assert!(!snapshot_text.contains("vec_secret"));
    assert!(!snapshot_text.contains("doc_secret"));
    assert!(!snapshot_text.contains("3f800000,3f000000,3e800000,3e000000"));

    let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let hits = reopened
        .knn_for_model(
            QueryVector::new(vec![1.0, 0.5, 0.25, 0.125]).unwrap(),
            1,
            "model-private",
        )
        .unwrap();
    assert_eq!(hits[0].doc_id(), "doc_secret");
    assert_eq!(
        inspect_persistent_vector_snapshot(&private_dir).state(),
        PersistentVectorSnapshotState::Ready
    );

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
fn persistent_vector_index_returns_requested_k_for_identical_hnsw_vectors() {
    let private_dir = temp_dir("private-identical-hnsw-vector-index");

    {
        let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        index
            .upsert(vec![
                VectorDocument::new_for_model(
                    "model-identical",
                    "model-identical:vec_alpha",
                    "doc_alpha",
                    vec![1.0, 0.0, 0.0, 0.0],
                )
                .unwrap(),
                VectorDocument::new_for_model(
                    "model-identical",
                    "model-identical:vec_bravo",
                    "doc_bravo",
                    vec![1.0, 0.0, 0.0, 0.0],
                )
                .unwrap(),
                VectorDocument::new_for_model(
                    "model-identical",
                    "model-identical:vec_charlie",
                    "doc_charlie",
                    vec![1.0, 0.0, 0.0, 0.0],
                )
                .unwrap(),
            ])
            .unwrap();
    }

    {
        let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        let hits = reopened
            .knn_for_model(
                QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
                3,
                "model-identical",
            )
            .unwrap();
        let doc_ids = hits
            .iter()
            .map(|hit| hit.doc_id().to_string())
            .collect::<Vec<_>>();
        assert_eq!(doc_ids, ["doc_alpha", "doc_bravo", "doc_charlie"]);
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
fn persistent_vector_index_merges_writes_from_stale_concurrent_openers() {
    let private_dir = temp_dir("private-vector-concurrent-writers");
    let first = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let second = PersistentVectorIndex::open(&private_dir, 4).unwrap();

    first
        .upsert(vec![VectorDocument::new(
            "vec_first",
            "doc_first",
            vec![1.0, 0.0, 0.0, 0.0],
        )
        .unwrap()])
        .unwrap();
    second
        .upsert(vec![VectorDocument::new(
            "vec_second",
            "doc_second",
            vec![0.0, 1.0, 0.0, 0.0],
        )
        .unwrap()])
        .unwrap();

    let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.vector_count(), 2);

    let first_hits = reopened
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 1)
        .unwrap();
    assert_eq!(first_hits[0].doc_id(), "doc_first");
    let second_hits = reopened
        .knn(QueryVector::new(vec![0.0, 1.0, 0.0, 0.0]).unwrap(), 1)
        .unwrap();
    assert_eq!(second_hits[0].doc_id(), "doc_second");
    let second_local_hits = second
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 1)
        .unwrap();
    assert_eq!(second_local_hits[0].doc_id(), "doc_first");

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_preserves_tombstones_from_stale_concurrent_openers() {
    let private_dir = temp_dir("private-vector-concurrent-tombstones");
    let seed = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    seed.upsert(vec![
        VectorDocument::new("vec_deleted", "doc_deleted", vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
        VectorDocument::new("vec_keep", "doc_keep", vec![0.0, 1.0, 0.0, 0.0]).unwrap(),
    ])
    .unwrap();

    let first = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let second = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    first.mark_deleted(&["vec_deleted"]).unwrap();
    second
        .upsert(vec![VectorDocument::new(
            "vec_new",
            "doc_new",
            vec![0.0, 0.0, 1.0, 0.0],
        )
        .unwrap()])
        .unwrap();

    let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.vector_count(), 3);
    assert_eq!(snapshot.deleted_count(), 1);

    let deleted_hits = reopened
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 3)
        .unwrap();
    assert!(deleted_hits.iter().all(|hit| hit.doc_id() != "doc_deleted"));
    let new_hits = reopened
        .knn(QueryVector::new(vec![0.0, 0.0, 1.0, 0.0]).unwrap(), 1)
        .unwrap();
    assert_eq!(new_hits[0].doc_id(), "doc_new");

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_recovers_last_good_snapshot_when_active_is_corrupt() {
    let private_dir = temp_dir("private-vector-last-good");

    {
        let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        index
            .upsert(vec![VectorDocument::new(
                "vec_recovered",
                "doc_recovered",
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap()])
            .unwrap();
        index
            .upsert(vec![VectorDocument::new(
                "vec_corrupt_active",
                "doc_corrupt_active",
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap()])
            .unwrap();
    }

    let backup_text = fs::read_to_string(private_dir.join("vector.snapshot.last-good")).unwrap();
    assert!(backup_text.starts_with("resume-ir-vector-index-encrypted-v1\n"));
    assert!(!backup_text.contains("doc_recovered"));
    assert!(!backup_text.contains("vec_recovered"));
    fs::write(
        private_dir.join("vector.snapshot"),
        "not a valid encrypted vector snapshot",
    )
    .unwrap();

    let inspection = inspect_persistent_vector_snapshot(&private_dir);
    assert_eq!(inspection.state(), PersistentVectorSnapshotState::Recovered);
    assert_eq!(
        inspection
            .snapshot()
            .map(|snapshot| snapshot.vector_count()),
        Some(1)
    );

    let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let recovered_hits = reopened
        .knn(QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(), 2)
        .unwrap();
    assert_eq!(recovered_hits.len(), 1);
    assert_eq!(recovered_hits[0].doc_id(), "doc_recovered");
    let corrupt_active_hits = reopened
        .knn(QueryVector::new(vec![0.0, 1.0, 0.0, 0.0]).unwrap(), 2)
        .unwrap();
    assert_eq!(corrupt_active_hits.len(), 1);
    assert_eq!(corrupt_active_hits[0].doc_id(), "doc_recovered");

    remove_dir(&private_dir);
}

#[test]
fn persistent_vector_index_recovers_last_good_snapshot_when_active_manifest_schema_mismatches() {
    let private_dir = temp_dir("private-vector-schema-mismatch");

    {
        let index = PersistentVectorIndex::open(&private_dir, 4).unwrap();
        index
            .upsert(vec![VectorDocument::new_for_model(
                "model-schema-v1",
                "model-schema-v1:vec_recovered",
                "doc_recovered",
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap()])
            .unwrap();
        index
            .upsert(vec![VectorDocument::new_for_model(
                "model-schema-v1",
                "model-schema-v1:vec_future_active",
                "doc_future_active",
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap()])
            .unwrap();
    }

    let manifest = fs::read_to_string(private_dir.join("vector.snapshot.manifest"))
        .expect("read vector snapshot manifest");
    assert!(manifest.contains("\"schema_version\":\"vector.snapshot.v1\""));
    assert!(manifest.contains("\"index_schema\":\"hnsw-vector.v1\""));
    assert!(manifest.contains("\"dimension\":4"));
    assert!(!manifest.contains("doc_recovered"));
    assert!(!manifest.contains("vec_recovered"));
    assert!(!manifest.contains("model-schema-v1"));
    assert!(!manifest.contains("1.0"));

    fs::write(
        private_dir.join("vector.snapshot.manifest"),
        "{\"schema_version\":\"vector.snapshot.v999\",\"index_schema\":\"future-vector-schema\",\"payload\":\"PRIVATE schema mismatch path\"}\n",
    )
    .unwrap();

    let inspection = inspect_persistent_vector_snapshot(&private_dir);
    assert_eq!(inspection.state(), PersistentVectorSnapshotState::Recovered);
    assert_eq!(
        inspection
            .snapshot()
            .map(|snapshot| snapshot.vector_count()),
        Some(1)
    );

    let reopened = PersistentVectorIndex::open(&private_dir, 4).unwrap();
    let recovered_hits = reopened
        .knn_for_model(
            QueryVector::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
            2,
            "model-schema-v1",
        )
        .unwrap();
    assert_eq!(recovered_hits.len(), 1);
    assert_eq!(recovered_hits[0].doc_id(), "doc_recovered");
    let debug = format!("{reopened:?}");
    assert!(!debug.contains("PRIVATE schema mismatch"));
    assert!(!debug.contains(path_str(&private_dir)));

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
