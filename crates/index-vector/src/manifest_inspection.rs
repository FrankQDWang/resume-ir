use std::fs;
use std::io::ErrorKind;

use crate::codec::{
    decode_manifest_metadata, KEY_FILE, MANIFEST_FILE, MAX_MANIFEST_BYTES, SNAPSHOT_FILE,
};
use crate::model::VectorIndexError;
use crate::model_contract::VectorModelContract;
use crate::private_storage::{PinnedPrivateDirectory, PinnedPrivateFile};
use crate::snapshot_model::VectorSnapshotManifestMetadata;
use crate::snapshot_root::{VectorSnapshotReadLease, VectorSnapshotRoot};
use crate::store::{require_regular_snapshot_directory, validate_generation, SNAPSHOTS_DIR};

impl VectorSnapshotRoot {
    /// Inspects one exact generation's bounded manifest without decrypting its
    /// payload, decoding vectors, or constructing an ANN index.
    ///
    /// The caller must retain the root-wide read lease. The generation
    /// directory plus manifest, encrypted payload, and key identities are
    /// pinned and revalidated before this method returns.
    pub fn inspect_generation_manifest_with_lease(
        &self,
        generation: &str,
        expected_model_contract: &VectorModelContract,
        lease: &VectorSnapshotReadLease,
    ) -> Result<Option<VectorSnapshotManifestMetadata>, VectorIndexError> {
        validate_generation(generation)?;
        expected_model_contract.validate()?;
        lease.validate_for(self)?;
        let snapshot_dir = self.root.join(SNAPSHOTS_DIR).join(generation);
        match fs::symlink_metadata(&snapshot_dir) {
            Ok(_) => require_regular_snapshot_directory(&snapshot_dir)?,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(VectorIndexError::Storage),
        }
        let generation_identity = PinnedPrivateDirectory::acquire(&snapshot_dir)?;
        let mut manifest = PinnedPrivateFile::acquire(
            &snapshot_dir.join(MANIFEST_FILE),
            Some(MAX_MANIFEST_BYTES),
        )?;
        let payload = PinnedPrivateFile::acquire(&snapshot_dir.join(SNAPSHOT_FILE), None)?;
        let key = PinnedPrivateFile::acquire(&snapshot_dir.join(KEY_FILE), None)?;

        validate_inspection_layout(self, lease, &generation_identity, &manifest, &payload, &key)?;
        let metadata = decode_manifest_metadata(
            &manifest.read_bounded()?,
            generation,
            expected_model_contract,
        )?;
        validate_inspection_layout(self, lease, &generation_identity, &manifest, &payload, &key)?;
        Ok(Some(metadata))
    }
}

fn validate_inspection_layout(
    owner: &VectorSnapshotRoot,
    lease: &VectorSnapshotReadLease,
    generation: &PinnedPrivateDirectory,
    manifest: &PinnedPrivateFile,
    payload: &PinnedPrivateFile,
    key: &PinnedPrivateFile,
) -> Result<(), VectorIndexError> {
    lease.validate_for(owner)?;
    generation.validate_current()?;
    manifest.validate_current()?;
    payload.validate_current()?;
    key.validate_current()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use serde_json::Value;

    use super::*;
    use crate::private_storage::write_private_bytes;
    use crate::store::VectorSnapshotStore;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const GENERATION: &str = "vector-manifest-inspection";

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        contract: VectorModelContract,
    }

    impl Fixture {
        fn published(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "resume-ir-vector-{label}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&base).unwrap();
            let root = base.join("index");
            let contract = VectorModelContract::Disabled;
            let store = VectorSnapshotStore::new(&root, contract.clone()).unwrap();
            store.publish_generation(GENERATION, [], []).unwrap();
            Self {
                base,
                root,
                contract,
            }
        }

        fn generation_path(&self) -> PathBuf {
            self.root.join(SNAPSHOTS_DIR).join(GENERATION)
        }

        fn inspect(
            &self,
            expected: &VectorModelContract,
        ) -> Result<Option<VectorSnapshotManifestMetadata>, VectorIndexError> {
            let root = VectorSnapshotRoot::new(&self.root)?;
            let lease = root.acquire_read_lease()?;
            root.inspect_generation_manifest_with_lease(GENERATION, expected, &lease)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn inspection_is_manifest_only_and_generation_bound() {
        let fixture = Fixture::published("manifest-only");
        fs::write(
            fixture.generation_path().join(SNAPSHOT_FILE),
            b"not valid ciphertext",
        )
        .unwrap();
        fs::write(fixture.generation_path().join(KEY_FILE), b"not a valid key").unwrap();

        let metadata = fixture.inspect(&fixture.contract).unwrap().unwrap();
        assert_eq!(metadata.generation(), GENERATION);
        assert_eq!(metadata.model_contract(), &fixture.contract);
        assert_eq!(metadata.vector_count(), 0);
        assert_eq!(metadata.projection_count(), 0);
        assert_eq!(metadata.vector_document_count(), 0);

        let root = VectorSnapshotRoot::new(&fixture.root).unwrap();
        let lease = root.acquire_read_lease().unwrap();
        assert_eq!(
            root.open_generation_with_lease(GENERATION, &fixture.contract, lease)
                .unwrap_err(),
            VectorIndexError::CorruptSnapshot,
        );
    }

    #[test]
    fn missing_generation_is_distinct_from_missing_generation_artifact() {
        let fixture = Fixture::published("missing");
        let root = VectorSnapshotRoot::new(&fixture.root).unwrap();
        let lease = root.acquire_read_lease().unwrap();
        assert_eq!(
            root.inspect_generation_manifest_with_lease(
                "vector-generation-missing",
                &fixture.contract,
                &lease,
            )
            .unwrap(),
            None,
        );

        let manifest_path = fixture.generation_path().join(MANIFEST_FILE);
        fs::remove_file(&manifest_path).unwrap();
        assert!(!manifest_path.exists());
        assert_eq!(
            fixture.inspect(&fixture.contract).unwrap_err(),
            VectorIndexError::CorruptSnapshot,
        );
    }

    #[test]
    fn manifest_rejects_oversize_unknown_keys_and_model_mismatch() {
        let oversized = Fixture::published("oversized");
        fs::write(
            oversized.generation_path().join(MANIFEST_FILE),
            vec![b'x'; MAX_MANIFEST_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            oversized.inspect(&oversized.contract).unwrap_err(),
            VectorIndexError::CorruptSnapshot,
        );

        let unknown = Fixture::published("unknown-key");
        let manifest_path = unknown.generation_path().join(MANIFEST_FILE);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .insert("legacy_alias".to_string(), Value::Bool(true));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            unknown.inspect(&unknown.contract).unwrap_err(),
            VectorIndexError::CorruptSnapshot,
        );

        let mismatch = Fixture::published("model-mismatch");
        let enabled = VectorModelContract::enabled("synthetic-model", 3).unwrap();
        assert_eq!(
            mismatch.inspect(&enabled).unwrap_err(),
            VectorIndexError::CorruptSnapshot,
        );
    }

    #[cfg(unix)]
    #[test]
    fn every_required_artifact_rejects_symlinks() {
        for (label, artifact) in [
            ("manifest", MANIFEST_FILE),
            ("payload", SNAPSHOT_FILE),
            ("key", KEY_FILE),
        ] {
            let fixture = Fixture::published(label);
            let artifact_path = fixture.generation_path().join(artifact);
            fs::remove_file(&artifact_path).unwrap();
            symlink(
                fixture.generation_path().join(MANIFEST_FILE),
                &artifact_path,
            )
            .unwrap();
            assert_eq!(
                fixture.inspect(&fixture.contract).unwrap_err(),
                VectorIndexError::StorageLayoutInvalid,
            );
        }
    }

    #[test]
    fn pinned_file_rejects_descriptor_path_replacement() {
        let fixture = Fixture::published("descriptor-mismatch");
        let manifest = fixture.generation_path().join(MANIFEST_FILE);
        let pinned = PinnedPrivateFile::acquire(&manifest, Some(MAX_MANIFEST_BYTES)).unwrap();
        fs::rename(
            &manifest,
            fixture.generation_path().join("displaced-manifest"),
        )
        .unwrap();
        write_private_bytes(&manifest, b"{}").unwrap();

        assert_eq!(
            pinned.validate_current().unwrap_err(),
            VectorIndexError::StorageLayoutInvalid,
        );
    }
}
