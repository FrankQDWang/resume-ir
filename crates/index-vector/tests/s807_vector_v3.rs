use core_domain::{ActiveSearchProjection, DocumentId, ResumeVersionId};
use index_vector::{
    commit_snapshot_gc, QueryVector, VectorDocument, VectorDocumentIdentity, VectorGenerationState,
    VectorIndexError, VectorModelContract, VectorSnapshotGcCommitReport,
    VectorSnapshotGcPreparation, VectorSnapshotReadLease, VectorSnapshotReader, VectorSnapshotRoot,
    VectorSnapshotStore as ProductionVectorSnapshotStore, VectorSnapshotSummary,
    VectorSnapshotUpdate, MAX_VECTOR_DIMENSION, VECTOR_SNAPSHOT_SCHEMA_V4,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

struct VectorSnapshotStore {
    writer: ProductionVectorSnapshotStore,
    root: VectorSnapshotRoot,
    contract: VectorModelContract,
}

impl VectorSnapshotStore {
    fn new(root: impl AsRef<Path>, dimension: usize) -> Result<Self, VectorIndexError> {
        let contract = VectorModelContract::enabled("model", dimension)?;
        Ok(Self {
            writer: ProductionVectorSnapshotStore::new(&root, contract.clone())?,
            root: VectorSnapshotRoot::new(root)?,
            contract,
        })
    }

    fn publish_generation<I>(
        &self,
        generation: &str,
        documents: I,
    ) -> Result<VectorSnapshotSummary, VectorIndexError>
    where
        I: IntoIterator<Item = VectorDocument>,
    {
        let documents = documents.into_iter().collect::<Vec<_>>();
        self.writer
            .publish_generation(generation, projection_for_documents(&documents), documents)
    }

    fn publish_generation_from(
        &self,
        base: VectorSnapshotReader,
        generation: &str,
        update: VectorSnapshotUpdate,
    ) -> Result<VectorSnapshotSummary, VectorIndexError> {
        self.writer
            .publish_generation_from(base, generation, update)
    }

    fn open_generation(&self, generation: &str) -> Result<VectorSnapshotReader, VectorIndexError> {
        let lease = self.root.acquire_read_lease()?;
        self.root
            .open_generation_with_lease(generation, &self.contract, lease)
    }

    fn acquire_read_lease(&self) -> Result<VectorSnapshotReadLease, VectorIndexError> {
        self.root.acquire_read_lease()
    }

    fn open_generation_with_lease(
        &self,
        generation: &str,
        lease: VectorSnapshotReadLease,
    ) -> Result<VectorSnapshotReader, VectorIndexError> {
        self.root
            .open_generation_with_lease(generation, &self.contract, lease)
    }

    fn inspect_generation(&self, generation: &str) -> index_vector::VectorGenerationInspection {
        let lease = self.root.acquire_read_lease().unwrap();
        self.root
            .inspect_generation_with_lease(generation, &self.contract, &lease)
    }

    fn garbage_collect(
        &self,
        retained: &BTreeSet<String>,
    ) -> Result<TestGcSummary, VectorIndexError> {
        let Some(acquisition) = self.root.try_acquire_snapshot_gc()? else {
            return Ok(TestGcSummary::Deferred);
        };
        let prepared = match self.root.prepare_snapshot_gc(acquisition, retained)? {
            VectorSnapshotGcPreparation::Deferred => return Ok(TestGcSummary::Deferred),
            VectorSnapshotGcPreparation::Prepared(prepared) => prepared,
        };
        Ok(match commit_snapshot_gc(prepared) {
            VectorSnapshotGcCommitReport::Complete(summary) => TestGcSummary::Completed(summary),
            VectorSnapshotGcCommitReport::Interrupted(_) => {
                panic!("vector GC unexpectedly interrupted")
            }
            VectorSnapshotGcCommitReport::PartialFailure(failure) => {
                TestGcSummary::Partial(failure)
            }
        })
    }
}

enum TestGcSummary {
    Deferred,
    Completed(index_vector::VectorGcSummary),
    Partial(index_vector::VectorGcPartialFailure),
}

impl TestGcSummary {
    fn is_deferred(&self) -> bool {
        matches!(self, Self::Deferred)
    }

    fn removed_generations(&self) -> usize {
        match self {
            Self::Deferred => 0,
            Self::Completed(summary) => summary.removed_generations(),
            Self::Partial(failure) => failure.progress().removed_generations(),
        }
    }
}

#[test]
fn publishes_and_opens_only_an_explicit_generation_with_exact_version_hits() {
    let root = temp_dir("exact-generation");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let java = document(
        "java",
        "candidate-a",
        "version-a",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let data = document(
        "data",
        "candidate-b",
        "version-b",
        "model",
        [0.0, 1.0, 0.0, 0.0],
    );

    let published = store
        .publish_generation("generation-a", [java.clone(), data])
        .unwrap();
    assert_eq!(published.generation(), "generation-a");
    assert_eq!(published.schema(), VECTOR_SNAPSHOT_SCHEMA_V4);
    assert_eq!(published.schema().manifest_schema(), "vector.snapshot.v4");
    assert_eq!(published.schema().index_schema(), "hnsw-vector.v4");
    assert_eq!(published.vector_count(), 2);
    assert_eq!(published.projection_count(), 2);
    assert_eq!(published.vector_document_count(), 2);
    assert_eq!(published.model_contract(), &store.contract);

    let reader = store.open_generation("generation-a").unwrap();
    assert_eq!(reader.documents_for_republication().len(), 2);
    let hits = reader.knn(query([1.0, 0.0, 0.0, 0.0]), 1).unwrap();
    assert_eq!(hits[0].document_id(), java.document_id());
    assert_eq!(hits[0].resume_version_id(), java.resume_version_id());
    assert_eq!(hits[0].model_id(), "model");
    assert_eq!(
        store.open_generation("generation-missing").unwrap_err(),
        VectorIndexError::GenerationNotFound
    );
    assert_eq!(
        store.inspect_generation("generation-a").state(),
        VectorGenerationState::Ready
    );
    remove_dir(&root);
}

#[test]
fn missing_generation_read_does_not_create_query_path_state() {
    let root = temp_dir("read-only-missing");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    assert_eq!(
        store.open_generation("generation-missing").unwrap_err(),
        VectorIndexError::GenerationNotFound
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    remove_dir(&root);
}

#[cfg(unix)]
#[test]
fn lock_files_are_owner_only_and_query_never_repairs_permissions() {
    let root = temp_dir("private-locks");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    store
        .publish_generation(
            "generation-private-locks",
            [document(
                "private-locks",
                "private-locks",
                "private-locks",
                "model",
                [1.0, 0.0, 0.0, 0.0],
            )],
        )
        .unwrap();
    for name in [
        "snapshot-readers.lock",
        "snapshot-publication.lock",
        "generation-pins/generation-private-locks.lock",
    ] {
        assert_eq!(
            fs::symlink_metadata(root.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let reader_lock = root.join("snapshot-readers.lock");
    fs::set_permissions(&reader_lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        store.acquire_read_lease(),
        Err(VectorIndexError::StorageLayoutInvalid)
    ));
    assert_eq!(
        fs::symlink_metadata(&reader_lock)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    remove_dir(&root);
}

#[cfg(unix)]
#[test]
fn publication_rejects_symlink_or_permissive_lock_files() {
    for (label, symlink_lock) in [("permissive", false), ("symlink", true)] {
        let root = temp_dir(&format!("publication-lock-{label}"));
        let lock = root.join("snapshot-publication.lock");
        if symlink_lock {
            let target = root.join("lock-target");
            fs::write(&target, b"synthetic").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
            symlink(&target, &lock).unwrap();
        } else {
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let store = VectorSnapshotStore::new(&root, 4).unwrap();
        assert_eq!(
            store
                .publish_generation(
                    "generation-rejected",
                    [document(label, label, label, "model", [1.0, 0.0, 0.0, 0.0],)],
                )
                .unwrap_err(),
            VectorIndexError::StorageLayoutInvalid
        );
        assert!(!root.join("snapshots/generation-rejected").exists());
        remove_dir(&root);
    }
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_generation_and_artifact_symlinks() {
    let root = temp_dir("artifact-symlinks");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    store
        .publish_generation(
            "generation-real",
            [document(
                "real",
                "real",
                "real",
                "model",
                [1.0, 0.0, 0.0, 0.0],
            )],
        )
        .unwrap();
    symlink(
        root.join("snapshots/generation-real"),
        root.join("snapshots/generation-alias"),
    )
    .unwrap();
    assert_eq!(
        store.open_generation("generation-alias").unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );

    for artifact in ["snapshot-manifest.json", "vector.snapshot.enc"] {
        let path = root.join("snapshots/generation-real").join(artifact);
        let backup = path.with_extension("synthetic-backup");
        fs::rename(&path, &backup).unwrap();
        symlink(&backup, &path).unwrap();
        assert_eq!(
            store.open_generation("generation-real").unwrap_err(),
            VectorIndexError::StorageLayoutInvalid
        );
        fs::remove_file(&path).unwrap();
        fs::rename(&backup, &path).unwrap();
    }

    let key = root.join("snapshots/generation-real/vector.snapshot.key-v4");
    let key_backup = root.join("snapshots/generation-real/vector.snapshot.key-v4.synthetic-backup");
    fs::rename(&key, &key_backup).unwrap();
    symlink(&key_backup, &key).unwrap();
    assert_eq!(
        store.open_generation("generation-real").unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    remove_dir(&root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_permissive_private_artifacts_without_chmod() {
    for relative in [
        "snapshots/generation-private/snapshot-manifest.json",
        "snapshots/generation-private/vector.snapshot.enc",
        "snapshots/generation-private/vector.snapshot.key-v4",
    ] {
        let root = temp_dir("artifact-permissions");
        let store = VectorSnapshotStore::new(&root, 4).unwrap();
        store
            .publish_generation(
                "generation-private",
                [document(
                    relative,
                    relative,
                    relative,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
        let path = root.join(relative);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            store.open_generation("generation-private").unwrap_err(),
            VectorIndexError::StorageLayoutInvalid
        );
        assert_eq!(
            fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        remove_dir(&root);
    }
}

#[cfg(unix)]
#[test]
fn damaged_generation_key_does_not_poison_later_publications() {
    let root = temp_dir("generation-local-vector-keys");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let publish = |generation: &str| {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    };
    publish("generation-a");
    let first_key = root.join("snapshots/generation-a/vector.snapshot.key-v4");
    let first_key_bytes = fs::read(&first_key).unwrap();
    fs::remove_file(&first_key).unwrap();

    publish("generation-b");
    let second_key = root.join("snapshots/generation-b/vector.snapshot.key-v4");
    let second_key_bytes = fs::read(&second_key).unwrap();
    assert_ne!(second_key_bytes, first_key_bytes);
    fs::write(&second_key, b"corrupt generation key").unwrap();

    publish("generation-c");
    let third_key = root.join("snapshots/generation-c/vector.snapshot.key-v4");
    let third_key_bytes = fs::read(&third_key).unwrap();
    assert_ne!(third_key_bytes, second_key_bytes);
    fs::set_permissions(&third_key, fs::Permissions::from_mode(0o644)).unwrap();

    publish("generation-d");
    let fourth_key = root.join("snapshots/generation-d/vector.snapshot.key-v4");
    let fourth_key_bytes = fs::read(&fourth_key).unwrap();
    assert_ne!(fourth_key_bytes, third_key_bytes);
    let fourth_backup = fourth_key.with_extension("backup");
    fs::rename(&fourth_key, &fourth_backup).unwrap();
    symlink(&fourth_backup, &fourth_key).unwrap();

    publish("generation-e");
    assert!(store.open_generation("generation-a").is_err());
    assert!(store.open_generation("generation-b").is_err());
    assert!(store.open_generation("generation-c").is_err());
    assert!(store.open_generation("generation-d").is_err());
    store.open_generation("generation-e").unwrap();
    remove_dir(&root);
}

#[test]
fn encrypted_v4_artifact_does_not_expose_identity_or_vector_values() {
    let root = temp_dir("encrypted");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let document = document(
        "private-vector",
        "private-document",
        "private-version",
        "model",
        [1.0, 0.5, 0.25, 0.125],
    );
    let private_ids = [
        document.vector_id().to_string(),
        document.document_id().to_string(),
        document.resume_version_id().to_string(),
        document.model_id().to_string(),
    ];
    store
        .publish_generation("generation-private", [document])
        .unwrap();

    let encrypted =
        fs::read_to_string(root.join("snapshots/generation-private/vector.snapshot.enc")).unwrap();
    assert!(encrypted.starts_with("resume-ir-vector-index-encrypted-v4\n"));
    for private_id in private_ids {
        assert!(!encrypted.contains(&private_id));
    }
    assert!(!encrypted.contains("3f800000"));
    remove_dir(&root);
}

#[test]
fn one_generation_rejects_two_versions_of_the_same_document() {
    let root = temp_dir("conflicting-version");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let version_a = document("a", "same-doc", "version-a", "model", [1.0, 0.0, 0.0, 0.0]);
    let version_b = document("b", "same-doc", "version-b", "model", [0.0, 1.0, 0.0, 0.0]);

    assert_eq!(
        store
            .publish_generation("generation-conflict", [version_a, version_b])
            .unwrap_err(),
        VectorIndexError::PublicationProjectionMismatch
    );
    assert!(!root.join("snapshots/generation-conflict").exists());
    assert!(!root.join("snapshot-readers.lock").exists());
    remove_dir(&root);
}

#[test]
fn separate_generations_never_mix_versions_for_the_same_document() {
    let root = temp_dir("version-isolation");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let version_a = document("a", "same-doc", "version-a", "model", [1.0, 0.0, 0.0, 0.0]);
    let version_b = document("b", "same-doc", "version-b", "model", [0.0, 1.0, 0.0, 0.0]);
    store
        .publish_generation("generation-a", [version_a.clone()])
        .unwrap();
    store
        .publish_generation("generation-b", [version_b.clone()])
        .unwrap();

    let hits_a = store
        .open_generation("generation-a")
        .unwrap()
        .knn(query([0.0, 1.0, 0.0, 0.0]), 1)
        .unwrap();
    let hits_b = store
        .open_generation("generation-b")
        .unwrap()
        .knn(query([1.0, 0.0, 0.0, 0.0]), 1)
        .unwrap();
    assert_eq!(hits_a[0].resume_version_id(), version_a.resume_version_id());
    assert_eq!(hits_b[0].resume_version_id(), version_b.resume_version_id());
    remove_dir(&root);
}

#[test]
fn failed_staging_validation_does_not_damage_an_existing_generation() {
    let root = temp_dir("staging-failure");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let stable = document("stable", "stable", "stable", "model", [1.0, 0.0, 0.0, 0.0]);
    store
        .publish_generation("generation-stable", [stable.clone()])
        .unwrap();
    let invalid = VectorDocument::new(
        identity("invalid", "invalid", "invalid", "model"),
        vec![1.0, 0.0],
    )
    .unwrap();

    assert_eq!(
        store
            .publish_generation("generation-invalid", [invalid])
            .unwrap_err(),
        VectorIndexError::InvalidDimension {
            expected: 4,
            actual: 2,
        }
    );
    assert_eq!(
        store
            .open_generation("generation-stable")
            .unwrap()
            .knn(query([1.0, 0.0, 0.0, 0.0]), 1)
            .unwrap()[0]
            .resume_version_id(),
        stable.resume_version_id()
    );
    assert_eq!(
        store.inspect_generation("generation-invalid").state(),
        VectorGenerationState::Missing
    );
    remove_dir(&root);
}

#[test]
fn corrupt_generation_fails_closed_without_last_good_fallback() {
    let root = temp_dir("corrupt-no-fallback");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    store
        .publish_generation(
            "generation-good",
            [document(
                "good",
                "good",
                "good",
                "model",
                [1.0, 0.0, 0.0, 0.0],
            )],
        )
        .unwrap();
    store
        .publish_generation(
            "generation-bad",
            [document("bad", "bad", "bad", "model", [0.0, 1.0, 0.0, 0.0])],
        )
        .unwrap();
    fs::write(
        root.join("snapshots/generation-bad/vector.snapshot.enc"),
        "resume-ir-vector-index-encrypted-v4\ninvalid\ninvalid\n",
    )
    .unwrap();

    assert_eq!(
        store.open_generation("generation-bad").unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    assert_eq!(
        store.inspect_generation("generation-bad").state(),
        VectorGenerationState::Corrupt
    );
    assert!(store.open_generation("generation-good").is_ok());
    remove_dir(&root);
}

#[test]
fn v3_manifest_is_incompatible_and_never_dual_read() {
    let root = temp_dir("v3-fail-closed");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    store
        .publish_generation(
            "generation-v3",
            [document("v3", "v3", "v3", "model", [1.0, 0.0, 0.0, 0.0])],
        )
        .unwrap();
    let manifest_path = root.join("snapshots/generation-v3/snapshot-manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["schema_version"] = serde_json::json!("vector.snapshot.v3");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    assert_eq!(
        store.open_generation("generation-v3").unwrap_err(),
        VectorIndexError::SchemaMismatch
    );
    assert_eq!(
        store.inspect_generation("generation-v3").state(),
        VectorGenerationState::Incompatible
    );
    remove_dir(&root);
}

#[test]
fn duplicate_generation_is_rejected_without_mutating_original() {
    let root = temp_dir("duplicate-generation");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let original = document(
        "original",
        "same",
        "original",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let replacement = document(
        "replacement",
        "other",
        "replacement",
        "model",
        [0.0, 1.0, 0.0, 0.0],
    );
    store
        .publish_generation("generation-fixed", [original.clone()])
        .unwrap();

    assert_eq!(
        store
            .publish_generation("generation-fixed", [replacement])
            .unwrap_err(),
        VectorIndexError::GenerationAlreadyExists
    );
    let hits = store
        .open_generation("generation-fixed")
        .unwrap()
        .knn(query([0.0, 1.0, 0.0, 0.0]), 1)
        .unwrap();
    assert_eq!(hits[0].resume_version_id(), original.resume_version_id());
    remove_dir(&root);
}

#[test]
fn retained_generation_reader_does_not_block_obsolete_generation_gc() {
    let root = temp_dir("reader-gc");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    for generation in ["generation-retained", "generation-old"] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    let reader = store.open_generation("generation-retained").unwrap();
    let retained = BTreeSet::from(["generation-retained".to_string()]);

    let completed = store.garbage_collect(&retained).unwrap();
    assert!(!completed.is_deferred());
    assert_eq!(completed.removed_generations(), 1);
    assert!(!root.join("snapshots/generation-old").exists());
    assert!(root.join("snapshots/generation-retained").exists());
    assert!(!root.join("generation-pins/generation-old.lock").exists());
    assert!(root
        .join("generation-pins/generation-retained.lock")
        .exists());
    assert_eq!(reader.generation(), "generation-retained");
    drop(reader);
    remove_dir(&root);
}

#[test]
fn obsolete_reader_defers_gc_without_blocking_current_query_or_publication() {
    let root = temp_dir("obsolete-reader-generation-gc");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    for generation in ["generation-retained", "generation-old"] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    let reader = store.open_generation("generation-old").unwrap();
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let deferred = store.garbage_collect(&retained).unwrap();
    assert!(deferred.is_deferred());
    assert!(root.join("snapshots/generation-old").exists());
    assert_eq!(
        store
            .open_generation("generation-retained")
            .unwrap()
            .generation(),
        "generation-retained"
    );
    let next = document("next", "next", "next", "model", [0.0, 1.0, 0.0, 0.0]);
    store.publish_generation("generation-next", [next]).unwrap();
    drop(reader);

    let retained = BTreeSet::from([
        "generation-retained".to_string(),
        "generation-next".to_string(),
    ]);
    assert_eq!(
        store
            .garbage_collect(&retained)
            .unwrap()
            .removed_generations(),
        1
    );
    assert!(!root.join("snapshots/generation-old").exists());
    assert!(!root.join("generation-pins/generation-old.lock").exists());
    remove_dir(&root);
}

#[test]
fn busy_late_candidate_defers_gc_without_deleting_an_earlier_candidate() {
    let root = temp_dir("busy-late-generation-reader-gc");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    for generation in [
        "generation-a-free",
        "generation-b-retained",
        "generation-z-busy",
    ] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    let busy_reader = store.open_generation("generation-z-busy").unwrap();
    let retained = BTreeSet::from(["generation-b-retained".to_string()]);

    assert!(store.garbage_collect(&retained).unwrap().is_deferred());
    for generation in ["generation-a-free", "generation-z-busy"] {
        assert!(root.join("snapshots").join(generation).exists());
        assert!(root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .exists());
    }

    drop(busy_reader);
    assert_eq!(
        store
            .garbage_collect(&retained)
            .unwrap()
            .removed_generations(),
        2
    );
    remove_dir(&root);
}

#[test]
fn exact_open_acquires_generation_pin_before_releasing_root_fence() {
    let root = temp_dir("generation-open-gc-fence");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    for generation in ["generation-retained", "generation-old"] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    let acquisition_lease = store.acquire_read_lease().unwrap();
    assert!(store.root.try_acquire_snapshot_gc().unwrap().is_none());
    let reader = store
        .open_generation_with_lease("generation-old", acquisition_lease)
        .unwrap();
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    assert!(store.garbage_collect(&retained).unwrap().is_deferred());
    assert!(root.join("snapshots/generation-old").exists());
    drop(reader);

    assert_eq!(
        store
            .garbage_collect(&retained)
            .unwrap()
            .removed_generations(),
        1
    );
    assert!(!root.join("snapshots/generation-old").exists());
    assert!(!root.join("generation-pins/generation-old.lock").exists());
    remove_dir(&root);
}

#[test]
fn missing_generation_pin_fails_closed_before_open_or_gc_deletion() {
    let root = temp_dir("missing-generation-pin");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    for generation in ["generation-retained", "generation-old"] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    fs::remove_file(root.join("generation-pins/generation-old.lock")).unwrap();

    assert_eq!(
        store.open_generation("generation-old").unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let acquisition = store.root.try_acquire_snapshot_gc().unwrap().unwrap();
    let error = match store.root.prepare_snapshot_gc(acquisition, &retained) {
        Err(error) => error,
        Ok(_) => panic!("missing generation pin unexpectedly prepared GC"),
    };
    assert_eq!(error, VectorIndexError::StorageLayoutInvalid);
    assert!(root.join("snapshots/generation-old").exists());
    assert!(root.join("snapshots/generation-retained").exists());
    remove_dir(&root);
}

#[test]
fn gc_removes_controlled_crash_staging_and_reports_it() {
    let root = temp_dir("crash-staging-gc");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let retained_document = document(
        "retained",
        "retained",
        "retained",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    store
        .publish_generation("generation-retained", [retained_document])
        .unwrap();
    let crash_staging = root
        .join("staging")
        .join("generation-crash.tmp-000000000000000000000000");
    create_private_test_directory(&crash_staging);

    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let acquisition = store.root.try_acquire_snapshot_gc().unwrap().unwrap();
    let VectorSnapshotGcPreparation::Prepared(prepared) = store
        .root
        .prepare_snapshot_gc(acquisition, &retained)
        .unwrap()
    else {
        panic!("GC unexpectedly deferred");
    };
    let VectorSnapshotGcCommitReport::Complete(summary) = commit_snapshot_gc(prepared) else {
        panic!("GC unexpectedly failed");
    };
    assert_eq!(summary.removed_generations(), 0);
    assert_eq!(summary.removed_staging(), 1);
    assert!(!crash_staging.exists());
    remove_dir(&root);
}

#[cfg(unix)]
#[test]
fn gc_rejects_staging_symlink_without_following_it() {
    let root = temp_dir("crash-staging-symlink");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let retained_document = document(
        "retained",
        "retained",
        "retained",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    store
        .publish_generation("generation-retained", [retained_document])
        .unwrap();
    let target = temp_dir("crash-staging-symlink-target");
    fs::write(target.join("sentinel"), b"keep").unwrap();
    symlink(
        &target,
        root.join("staging/generation-crash.tmp-000000000000000000000000"),
    )
    .unwrap();

    let retained = BTreeSet::from(["generation-retained".to_string()]);
    let acquisition = store.root.try_acquire_snapshot_gc().unwrap().unwrap();
    assert!(store
        .root
        .prepare_snapshot_gc(acquisition, &retained)
        .is_err());
    assert!(target.join("sentinel").exists());
    remove_dir(&root);
    remove_dir(&target);
}

#[test]
fn preselected_lease_opens_the_exact_generation_and_rejects_other_store() {
    let first_root = temp_dir("lease-first");
    let second_root = temp_dir("lease-second");
    let first = VectorSnapshotStore::new(&first_root, 4).unwrap();
    let second = VectorSnapshotStore::new(&second_root, 4).unwrap();
    for (store, generation) in [(&first, "first"), (&second, "second")] {
        store
            .publish_generation(
                generation,
                [document(
                    generation,
                    generation,
                    generation,
                    "model",
                    [1.0, 0.0, 0.0, 0.0],
                )],
            )
            .unwrap();
    }
    let lease = first.acquire_read_lease().unwrap();
    assert_eq!(
        second
            .open_generation_with_lease("second", lease)
            .unwrap_err(),
        VectorIndexError::LeaseRootMismatch
    );
    let lease = first.acquire_read_lease().unwrap();
    assert_eq!(
        first
            .open_generation_with_lease("first", lease)
            .unwrap()
            .generation(),
        "first"
    );
    remove_dir(&first_root);
    remove_dir(&second_root);
}

#[test]
fn exact_base_update_retains_only_active_versions_and_applies_replacements() {
    let root = temp_dir("exact-base-update");
    let store = VectorSnapshotStore::new(&root, 4).unwrap();
    let stable = document(
        "stable",
        "stable",
        "stable-v1",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let old = document(
        "changing",
        "changing",
        "changing-v1",
        "model",
        [0.0, 1.0, 0.0, 0.0],
    );
    let deleted = document(
        "deleted",
        "deleted",
        "deleted-v1",
        "model",
        [0.0, 0.0, 1.0, 0.0],
    );
    store
        .publish_generation("generation-base", [stable.clone(), old, deleted])
        .unwrap();
    let base = store.open_generation("generation-base").unwrap();
    let replacement = document(
        "changing-v2",
        "changing",
        "changing-v2",
        "model",
        [0.0, 0.0, 0.0, 1.0],
    );
    let active_versions = BTreeMap::from([
        (
            stable.document_id().to_string(),
            stable.resume_version_id().to_string(),
        ),
        (
            replacement.document_id().to_string(),
            replacement.resume_version_id().to_string(),
        ),
    ]);
    let update = VectorSnapshotUpdate::new(
        projection_from_map(active_versions),
        vec![replacement.clone()],
        BTreeSet::new(),
    )
    .unwrap();
    store
        .publish_generation_from(base, "generation-next", update)
        .unwrap();

    let next = store.open_generation("generation-next").unwrap();
    assert_eq!(next.summary().vector_count(), 2);
    let hits = next.knn(query([0.0, 0.0, 0.0, 1.0]), 2).unwrap();
    assert!(hits.iter().any(|hit| {
        hit.document_id() == stable.document_id()
            && hit.resume_version_id() == stable.resume_version_id()
    }));
    assert!(hits.iter().any(|hit| {
        hit.document_id() == replacement.document_id()
            && hit.resume_version_id() == replacement.resume_version_id()
    }));
    assert!(hits
        .iter()
        .all(|hit| hit.document_id() != stable_id("doc_", "deleted")));
    remove_dir(&root);
}

#[test]
fn exact_base_update_rejects_replacement_outside_active_projection() {
    let replacement = document("replacement", "doc", "v2", "model", [1.0, 0.0, 0.0, 0.0]);
    let active_versions = BTreeMap::from([(
        replacement.document_id().to_string(),
        stable_id("ver_", "v1"),
    )]);
    assert_eq!(
        VectorSnapshotUpdate::new(
            projection_from_map(active_versions),
            vec![replacement],
            BTreeSet::new(),
        )
        .unwrap_err(),
        VectorIndexError::PublicationProjectionMismatch
    );
}

#[test]
fn disabled_generation_binds_complete_projection_without_fake_model_or_dimension() {
    let root = temp_dir("disabled-complete-projection");
    let contract = VectorModelContract::Disabled;
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let projection = vec![projection_entry("vectorless", "vectorless-v1")];
    let published = writer
        .publish_generation(
            "generation-disabled",
            projection.clone(),
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
    assert_eq!(published.projection_count(), 1);
    assert_eq!(published.vector_document_count(), 0);
    assert_eq!(published.model_contract(), &VectorModelContract::Disabled);

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("snapshots/generation-disabled/snapshot-manifest.json")).unwrap(),
    )
    .unwrap();
    assert!(manifest["model_id"].is_null());
    assert!(manifest["dimension"].is_null());

    let reader = open_exact_generation(&root, "generation-disabled", &contract).unwrap();
    assert_eq!(reader.exact_projection().len(), 1);
    assert_eq!(
        reader.knn(query([1.0, 0.0, 0.0, 0.0]), 1).unwrap_err(),
        VectorIndexError::SemanticUnavailable
    );
    remove_dir(&root);
}

#[test]
fn model_contract_rejects_dimensions_above_the_explicit_limit() {
    assert_eq!(
        VectorModelContract::enabled("model", MAX_VECTOR_DIMENSION + 1).unwrap_err(),
        VectorIndexError::InvalidDimension {
            expected: MAX_VECTOR_DIMENSION,
            actual: MAX_VECTOR_DIMENSION + 1,
        }
    );
}

#[test]
fn enabled_generation_projection_includes_documents_without_vectors() {
    let root = temp_dir("vectorless-active-document");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let embedded = document(
        "embedded",
        "embedded",
        "embedded-v1",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let projection = vec![
        projection_entry("vectorless", "vectorless-v1"),
        projection_entry("embedded", "embedded-v1"),
    ];
    let mut expected_projection = projection.clone();
    expected_projection.sort_unstable_by(|left, right| {
        left.document_id
            .cmp(&right.document_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });
    let summary = writer
        .publish_generation("generation-partial", projection, [embedded])
        .unwrap();
    assert_eq!(summary.projection_count(), 2);
    assert_eq!(summary.vector_document_count(), 1);
    assert_ne!(summary.projection_digest(), summary.coverage_digest());
    let reader = open_exact_generation(&root, "generation-partial", &contract).unwrap();
    let first_projection = reader.exact_projection();
    let second_projection = reader.exact_projection();
    assert_eq!(first_projection, expected_projection);
    assert!(std::ptr::eq(first_projection, second_projection));
    remove_dir(&root);
}

#[test]
fn exact_open_requires_the_metadata_selected_model_contract() {
    let root = temp_dir("exact-model-contract");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    writer
        .publish_generation(
            "generation-contract",
            Vec::new(),
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
    assert_eq!(
        open_exact_generation(
            &root,
            "generation-contract",
            &VectorModelContract::enabled("model", 8).unwrap(),
        )
        .unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    assert_eq!(
        open_exact_generation(&root, "generation-contract", &VectorModelContract::Disabled,)
            .unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    remove_dir(&root);
}

#[test]
fn logical_and_projection_digests_are_version_exact_and_empty_stable() {
    let root = temp_dir("vector-digest-identity");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let version_a = document(
        "vector-a",
        "same-document",
        "version-a",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let version_b = document(
        "vector-b",
        "same-document",
        "version-b",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    let first = writer
        .publish_generation(
            "generation-version-a",
            projection_for_documents(std::slice::from_ref(&version_a)),
            [version_a],
        )
        .unwrap();
    let second = writer
        .publish_generation(
            "generation-version-b",
            projection_for_documents(std::slice::from_ref(&version_b)),
            [version_b],
        )
        .unwrap();
    assert_ne!(first.projection_digest(), second.projection_digest());
    assert_ne!(
        first.logical_content_digest(),
        second.logical_content_digest()
    );

    let empty_a = writer
        .publish_generation(
            "generation-empty-a",
            Vec::new(),
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
    let empty_b = writer
        .publish_generation(
            "generation-empty-b",
            Vec::new(),
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
    assert_eq!(empty_a.projection_digest(), empty_b.projection_digest());
    assert_eq!(
        empty_a.logical_content_digest(),
        empty_b.logical_content_digest()
    );
    remove_dir(&root);
}

#[test]
fn manifest_identity_or_artifact_digest_tampering_fails_closed() {
    for field in [
        "projection_digest",
        "coverage_digest",
        "logical_content_digest",
        "artifact_digest",
    ] {
        let root = temp_dir(&format!("vector-{field}-tamper"));
        let contract = VectorModelContract::enabled("model", 4).unwrap();
        let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
        let document = document(field, field, field, "model", [1.0, 0.0, 0.0, 0.0]);
        let generation = format!("generation-{field}");
        writer
            .publish_generation(
                &generation,
                projection_for_documents(std::slice::from_ref(&document)),
                [document],
            )
            .unwrap();
        let manifest_path = root
            .join("snapshots")
            .join(&generation)
            .join("snapshot-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest[field] = serde_json::json!(format!("sha256:{}", "0".repeat(64)));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            open_exact_generation(&root, &generation, &contract).unwrap_err(),
            VectorIndexError::CorruptSnapshot
        );
        remove_dir(&root);
    }
}

#[test]
fn encrypted_vector_snapshot_byte_tampering_fails_closed() {
    let root = temp_dir("vector-encrypted-byte-tamper");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document = document(
        "tamper",
        "tamper",
        "tamper-v1",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    writer
        .publish_generation(
            "generation-byte-tamper",
            projection_for_documents(std::slice::from_ref(&document)),
            [document],
        )
        .unwrap();
    let encrypted_path = root.join("snapshots/generation-byte-tamper/vector.snapshot.enc");
    let mut encrypted = fs::read(&encrypted_path).unwrap();
    let ciphertext_byte = encrypted.len() - 2;
    encrypted[ciphertext_byte] = if encrypted[ciphertext_byte] == b'0' {
        b'1'
    } else {
        b'0'
    };
    fs::write(&encrypted_path, encrypted).unwrap();

    assert_eq!(
        open_exact_generation(&root, "generation-byte-tamper", &contract).unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    remove_dir(&root);
}

#[test]
fn oversized_vector_manifest_fails_closed_before_json_parse() {
    let root = temp_dir("vector-oversized-manifest");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document = document(
        "oversized",
        "oversized",
        "oversized-v1",
        "model",
        [1.0, 0.0, 0.0, 0.0],
    );
    writer
        .publish_generation(
            "generation-oversized",
            projection_for_documents(std::slice::from_ref(&document)),
            [document],
        )
        .unwrap();
    fs::write(
        root.join("snapshots/generation-oversized/snapshot-manifest.json"),
        vec![b'x'; 4 * 1024 + 1],
    )
    .unwrap();

    assert_eq!(
        open_exact_generation(&root, "generation-oversized", &contract).unwrap_err(),
        VectorIndexError::CorruptSnapshot
    );
    remove_dir(&root);
}

#[test]
fn writer_creates_a_fresh_owner_only_root() {
    let parent = temp_dir("fresh-root-parent");
    let root = parent.join("vector-index");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    ProductionVectorSnapshotStore::new(&root, contract).unwrap();
    assert!(root.is_dir());
    #[cfg(unix)]
    assert_eq!(
        fs::symlink_metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn canonical_root_pins_leases_across_symlink_retarget() {
    let parent = temp_dir("canonical-root-link-parent");
    let target_a = temp_dir("canonical-root-a");
    let target_b = temp_dir("canonical-root-b");
    let link = parent.join("vector-index");
    symlink(&target_a, &link).unwrap();
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer_a = ProductionVectorSnapshotStore::new(&link, contract.clone()).unwrap();
    let document_a = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);
    writer_a
        .publish_generation(
            "generation-a",
            projection_for_documents(std::slice::from_ref(&document_a)),
            [document_a],
        )
        .unwrap();
    let writer_b = ProductionVectorSnapshotStore::new(&target_b, contract.clone()).unwrap();
    let document_b = document("b", "b", "b", "model", [0.0, 1.0, 0.0, 0.0]);
    writer_b
        .publish_generation(
            "generation-b",
            projection_for_documents(std::slice::from_ref(&document_b)),
            [document_b],
        )
        .unwrap();

    let root_a = VectorSnapshotRoot::new(&link).unwrap();
    let lease_a = root_a.acquire_read_lease().unwrap();
    fs::remove_file(&link).unwrap();
    symlink(&target_b, &link).unwrap();
    assert!(root_a
        .open_generation_with_lease("generation-a", &contract, lease_a)
        .is_ok());
    let mismatched_lease = root_a.acquire_read_lease().unwrap();
    let root_b = VectorSnapshotRoot::new(&link).unwrap();
    assert_eq!(
        root_b
            .open_generation_with_lease("generation-b", &contract, mismatched_lease)
            .unwrap_err(),
        VectorIndexError::LeaseRootMismatch
    );
    assert!(target_b.join("snapshots/generation-b").exists());
    remove_dir(&parent);
    remove_dir(&target_a);
    remove_dir(&target_b);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_snapshots_intermediate_symlink_after_lease_acquisition() {
    let root = temp_dir("snapshots-intermediate-symlink");
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);
    writer
        .publish_generation(
            "generation-a",
            projection_for_documents(std::slice::from_ref(&document)),
            [document],
        )
        .unwrap();
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let lease = snapshot_root.acquire_read_lease().unwrap();
    let snapshots = root.join("snapshots");
    let real_snapshots = root.join("snapshots-real");
    fs::rename(&snapshots, &real_snapshots).unwrap();
    symlink(&real_snapshots, &snapshots).unwrap();

    assert_eq!(
        snapshot_root
            .open_generation_with_lease("generation-a", &contract, lease)
            .unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    assert!(real_snapshots.join("generation-a").exists());
    remove_dir(&root);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_same_path_root_inode_replacement() {
    let parent = temp_dir("vector-root-inode-replacement-parent");
    let root = parent.join("index");
    create_private_test_directory(&root);
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer_a = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document_a = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);
    writer_a
        .publish_generation(
            "generation-a",
            projection_for_documents(std::slice::from_ref(&document_a)),
            [document_a],
        )
        .unwrap();
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let lease = snapshot_root.acquire_read_lease().unwrap();
    let displaced = parent.join("index-displaced");
    fs::rename(&root, &displaced).unwrap();
    create_private_test_directory(&root);
    let writer_b = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document_b = document("b", "b", "b", "model", [0.0, 1.0, 0.0, 0.0]);
    writer_b
        .publish_generation(
            "generation-b",
            projection_for_documents(std::slice::from_ref(&document_b)),
            [document_b],
        )
        .unwrap();

    assert_eq!(
        snapshot_root
            .open_generation_with_lease("generation-b", &contract, lease)
            .unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    assert!(open_exact_generation(&root, "generation-b", &contract).is_ok());
    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn exact_open_rejects_intermediate_directory_inode_replacement() {
    for component in ["snapshots", "generation-pins"] {
        let root = temp_dir(&format!("vector-{component}-inode-replacement"));
        let contract = VectorModelContract::enabled("model", 4).unwrap();
        let writer = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
        let document = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);
        writer
            .publish_generation(
                "generation-a",
                projection_for_documents(std::slice::from_ref(&document)),
                [document],
            )
            .unwrap();
        let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
        let lease = snapshot_root.acquire_read_lease().unwrap();
        let original = root.join(component);
        let displaced = root.join(format!("{component}-displaced"));
        fs::rename(&original, &displaced).unwrap();
        create_private_test_directory(&original);
        if component == "snapshots" {
            fs::rename(
                displaced.join("generation-a"),
                original.join("generation-a"),
            )
            .unwrap();
        } else {
            fs::rename(
                displaced.join("generation-a.lock"),
                original.join("generation-a.lock"),
            )
            .unwrap();
        }

        assert_eq!(
            snapshot_root
                .open_generation_with_lease("generation-a", &contract, lease)
                .unwrap_err(),
            VectorIndexError::StorageLayoutInvalid
        );
        remove_dir(&root);
    }
}

#[cfg(unix)]
#[test]
fn gc_lease_rejects_same_path_root_inode_replacement_without_deleting_new_root() {
    let parent = temp_dir("vector-gc-root-inode-replacement-parent");
    let root = parent.join("index");
    create_private_test_directory(&root);
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer_a = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let document_a = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);
    writer_a
        .publish_generation(
            "generation-a",
            projection_for_documents(std::slice::from_ref(&document_a)),
            [document_a],
        )
        .unwrap();
    let snapshot_root = VectorSnapshotRoot::new(&root).unwrap();
    let gc_acquisition = snapshot_root.try_acquire_snapshot_gc().unwrap().unwrap();
    let displaced = parent.join("index-displaced");
    fs::rename(&root, &displaced).unwrap();
    create_private_test_directory(&root);
    let writer_b = ProductionVectorSnapshotStore::new(&root, contract).unwrap();
    let document_b = document("b", "b", "b", "model", [0.0, 1.0, 0.0, 0.0]);
    writer_b
        .publish_generation(
            "generation-b",
            projection_for_documents(std::slice::from_ref(&document_b)),
            [document_b],
        )
        .unwrap();

    let error = match snapshot_root.prepare_snapshot_gc(gc_acquisition, &BTreeSet::new()) {
        Err(error) => error,
        Ok(_) => panic!("replaced GC root unexpectedly prepared"),
    };
    assert_eq!(error, VectorIndexError::StorageLayoutInvalid);
    assert!(root.join("snapshots/generation-b").exists());
    remove_dir(&parent);
}

#[cfg(unix)]
#[test]
fn writer_rejects_same_path_root_inode_replacement() {
    let parent = temp_dir("vector-writer-root-inode-replacement-parent");
    let root = parent.join("index");
    create_private_test_directory(&root);
    let contract = VectorModelContract::enabled("model", 4).unwrap();
    let writer_a = ProductionVectorSnapshotStore::new(&root, contract.clone()).unwrap();
    let displaced = parent.join("index-displaced");
    fs::rename(&root, &displaced).unwrap();
    create_private_test_directory(&root);
    let writer_b = ProductionVectorSnapshotStore::new(&root, contract).unwrap();
    let document_b = document("b", "b", "b", "model", [0.0, 1.0, 0.0, 0.0]);
    writer_b
        .publish_generation(
            "generation-b",
            projection_for_documents(std::slice::from_ref(&document_b)),
            [document_b],
        )
        .unwrap();
    let document_a = document("a", "a", "a", "model", [1.0, 0.0, 0.0, 0.0]);

    assert_eq!(
        writer_a
            .publish_generation(
                "generation-a",
                projection_for_documents(std::slice::from_ref(&document_a)),
                [document_a],
            )
            .unwrap_err(),
        VectorIndexError::StorageLayoutInvalid
    );
    assert!(root.join("snapshots/generation-b").exists());
    assert!(!root.join("snapshots/generation-a").exists());
    remove_dir(&parent);
}

fn document(
    vector: &str,
    document: &str,
    version: &str,
    model: &str,
    values: [f32; 4],
) -> VectorDocument {
    VectorDocument::new(identity(vector, document, version, model), values.to_vec()).unwrap()
}

fn identity(vector: &str, document: &str, version: &str, model: &str) -> VectorDocumentIdentity {
    VectorDocumentIdentity::new(
        stable_id("vec_", vector),
        stable_id("doc_", document),
        stable_id("ver_", version),
        model,
    )
    .unwrap()
}

fn stable_id(prefix: &str, part: &str) -> String {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x6c62_272e_07bb_0142_u64;
    for byte in part.bytes() {
        first = (first ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        second = (second ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{first:016x}{second:016x}")
}

fn projection_for_documents(documents: &[VectorDocument]) -> Vec<ActiveSearchProjection> {
    documents
        .iter()
        .map(|document| {
            (
                document.document_id().to_string(),
                document.resume_version_id().to_string(),
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(document_id, resume_version_id)| ActiveSearchProjection {
            document_id: DocumentId::from_str(&document_id).unwrap(),
            resume_version_id: ResumeVersionId::from_str(&resume_version_id).unwrap(),
        })
        .collect()
}

fn projection_from_map(active_versions: BTreeMap<String, String>) -> Vec<ActiveSearchProjection> {
    active_versions
        .into_iter()
        .map(|(document_id, resume_version_id)| ActiveSearchProjection {
            document_id: DocumentId::from_str(&document_id).unwrap(),
            resume_version_id: ResumeVersionId::from_str(&resume_version_id).unwrap(),
        })
        .collect()
}

fn projection_entry(document: &str, version: &str) -> ActiveSearchProjection {
    ActiveSearchProjection {
        document_id: DocumentId::from_str(&stable_id("doc_", document)).unwrap(),
        resume_version_id: ResumeVersionId::from_str(&stable_id("ver_", version)).unwrap(),
    }
}

fn query(values: [f32; 4]) -> QueryVector {
    QueryVector::new(values.to_vec()).unwrap()
}

fn open_exact_generation(
    root: &Path,
    generation: &str,
    contract: &VectorModelContract,
) -> Result<VectorSnapshotReader, VectorIndexError> {
    let snapshot_root = VectorSnapshotRoot::new(root)?;
    let lease = snapshot_root.acquire_read_lease()?;
    snapshot_root.open_generation_with_lease(generation, contract, lease)
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-vector-v3-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn create_private_test_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
