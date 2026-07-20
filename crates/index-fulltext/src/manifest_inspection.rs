use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{
    decode_manifest, map_manifest_error, same_open_file_identity, snapshot_directory_exists,
    validate_regular_file_metadata, validate_regular_file_size, validate_snapshot_name,
    FilePrivacy, FullTextError, FullTextIndex, PinnedSnapshotDirectory, PublishedSnapshotMetadata,
    Result, SnapshotReadLease, ENCRYPTED_SNAPSHOT_FILE, MAX_MANIFEST_BYTES,
    MAX_SNAPSHOT_KEY_FILE_BYTES, SNAPSHOTS_DIR, SNAPSHOT_KEY_FILE, SNAPSHOT_MANIFEST_FILE,
};

struct PinnedOwnerOnlyFile {
    path: PathBuf,
    file: File,
    max_bytes: Option<usize>,
}

impl PinnedOwnerOnlyFile {
    fn acquire(path: &Path, max_bytes: Option<usize>) -> Result<Self> {
        let before = fs::symlink_metadata(path).map_err(FullTextError::io)?;
        validate_regular_file_metadata(&before, FilePrivacy::OwnerOnly)
            .map_err(FullTextError::io)?;
        validate_regular_file_size(&before, max_bytes).map_err(FullTextError::io)?;
        let file = File::open(path).map_err(FullTextError::io)?;
        let pinned = Self {
            path: path.to_path_buf(),
            file,
            max_bytes,
        };
        pinned.validate_current()?;
        Ok(pinned)
    }

    fn validate_current(&self) -> Result<()> {
        let opened = self.file.metadata().map_err(FullTextError::io)?;
        validate_regular_file_metadata(&opened, FilePrivacy::OwnerOnly)
            .map_err(FullTextError::io)?;
        validate_regular_file_size(&opened, self.max_bytes).map_err(FullTextError::io)?;
        let current = fs::symlink_metadata(&self.path).map_err(FullTextError::io)?;
        validate_regular_file_metadata(&current, FilePrivacy::OwnerOnly)
            .map_err(FullTextError::io)?;
        validate_regular_file_size(&current, self.max_bytes).map_err(FullTextError::io)?;
        if same_open_file_identity(&self.file, &self.path, &opened, &current)
            .map_err(FullTextError::io)?
        {
            Ok(())
        } else {
            Err(FullTextError::internal(
                "full-text inspection file identity changed",
            ))
        }
    }

    fn read_bounded(&mut self) -> Result<Vec<u8>> {
        let max_bytes = self
            .max_bytes
            .ok_or_else(|| FullTextError::internal("full-text inspection file is not bounded"))?;
        self.validate_current()?;
        let mut bytes = Vec::with_capacity(
            usize::try_from(self.file.metadata().map_err(FullTextError::io)?.len())
                .unwrap_or(usize::MAX)
                .min(max_bytes),
        );
        (&mut self.file)
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(FullTextError::io)?;
        if bytes.len() > max_bytes {
            return Err(FullTextError::internal(
                "full-text snapshot manifest corrupt",
            ));
        }
        self.validate_current()?;
        Ok(bytes)
    }
}

impl FullTextIndex {
    /// Inspects one exact generation's bounded manifest without decrypting or
    /// extracting its payload and without opening Tantivy.
    ///
    /// The caller must retain the root-wide read lease. The generation
    /// directory plus manifest, encrypted payload, and key identities are
    /// pinned and revalidated before this method returns.
    pub fn inspect_snapshot_manifest_with_lease(
        index_root: &Path,
        snapshot_name: &str,
        lease: &SnapshotReadLease,
    ) -> Result<Option<PublishedSnapshotMetadata>> {
        validate_snapshot_name(snapshot_name)?;
        if !lease.protects(index_root)? {
            return Err(FullTextError::internal(
                "full-text snapshot lease belongs to another index root",
            ));
        }
        let pinned_root = lease.index_root.clone();
        lease.validate_layout()?;
        let snapshot_dir = pinned_root.join(SNAPSHOTS_DIR).join(snapshot_name);
        if !snapshot_directory_exists(&snapshot_dir)? {
            return Ok(None);
        }
        let generation = PinnedSnapshotDirectory::acquire(&snapshot_dir)?;
        let mut manifest = PinnedOwnerOnlyFile::acquire(
            &snapshot_dir.join(SNAPSHOT_MANIFEST_FILE),
            Some(MAX_MANIFEST_BYTES),
        )?;
        let payload =
            PinnedOwnerOnlyFile::acquire(&snapshot_dir.join(ENCRYPTED_SNAPSHOT_FILE), None)?;
        let key = PinnedOwnerOnlyFile::acquire(
            &snapshot_dir.join(SNAPSHOT_KEY_FILE),
            Some(MAX_SNAPSHOT_KEY_FILE_BYTES),
        )?;

        validate_inspection_layout(lease, &generation, &manifest, &payload, &key)?;
        let metadata = decode_manifest(&manifest.read_bounded()?, snapshot_name)
            .map_err(map_manifest_error)?;
        validate_inspection_layout(lease, &generation, &manifest, &payload, &key)?;
        Ok(Some(metadata))
    }
}

fn validate_inspection_layout(
    lease: &SnapshotReadLease,
    generation: &PinnedSnapshotDirectory,
    manifest: &PinnedOwnerOnlyFile,
    payload: &PinnedOwnerOnlyFile,
    key: &PinnedOwnerOnlyFile,
) -> Result<()> {
    lease.validate_layout()?;
    generation.validate_current()?;
    manifest.validate_current()?;
    payload.validate_current()?;
    key.validate_current()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use serde_json::Value;

    use super::*;
    use crate::publish_snapshot;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    const GENERATION: &str = "fulltext-manifest-inspection";

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
    }

    impl Fixture {
        fn published(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "resume-ir-fulltext-{label}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&base).unwrap();
            let root = base.join("index");
            publish_snapshot(&root, GENERATION, Vec::new()).unwrap();
            Self { base, root }
        }

        fn generation_path(&self) -> PathBuf {
            self.root.join(SNAPSHOTS_DIR).join(GENERATION)
        }

        fn inspect(&self) -> Result<Option<PublishedSnapshotMetadata>> {
            let lease = SnapshotReadLease::acquire(&self.root)?.unwrap();
            FullTextIndex::inspect_snapshot_manifest_with_lease(&self.root, GENERATION, &lease)
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
            fixture.generation_path().join(ENCRYPTED_SNAPSHOT_FILE),
            b"not valid ciphertext",
        )
        .unwrap();
        fs::write(
            fixture.generation_path().join(SNAPSHOT_KEY_FILE),
            b"not a valid key",
        )
        .unwrap();

        let metadata = fixture.inspect().unwrap().unwrap();
        assert_eq!(metadata.generation(), GENERATION);
        assert_eq!(metadata.document_count(), 0);

        let lease = SnapshotReadLease::acquire(&fixture.root).unwrap().unwrap();
        assert!(FullTextIndex::open_snapshot_with_lease(&fixture.root, GENERATION, lease).is_err());
    }

    #[test]
    fn missing_generation_is_distinct_from_missing_manifest() {
        let fixture = Fixture::published("missing");
        let lease = SnapshotReadLease::acquire(&fixture.root).unwrap().unwrap();
        assert_eq!(
            FullTextIndex::inspect_snapshot_manifest_with_lease(
                &fixture.root,
                "fulltext-generation-missing",
                &lease,
            )
            .unwrap(),
            None,
        );

        fs::remove_file(fixture.generation_path().join(SNAPSHOT_MANIFEST_FILE)).unwrap();
        assert!(fixture.inspect().is_err());
    }

    #[test]
    fn manifest_rejects_oversize_and_unknown_keys() {
        let oversized = Fixture::published("oversized");
        fs::write(
            oversized.generation_path().join(SNAPSHOT_MANIFEST_FILE),
            vec![b'x'; MAX_MANIFEST_BYTES + 1],
        )
        .unwrap();
        assert!(oversized.inspect().is_err());

        let unknown = Fixture::published("unknown-key");
        let manifest_path = unknown.generation_path().join(SNAPSHOT_MANIFEST_FILE);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest
            .as_object_mut()
            .unwrap()
            .insert("legacy_alias".to_string(), Value::Bool(true));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(unknown.inspect().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn every_required_artifact_rejects_symlinks() {
        for (label, artifact) in [
            ("manifest", SNAPSHOT_MANIFEST_FILE),
            ("payload", ENCRYPTED_SNAPSHOT_FILE),
            ("key", SNAPSHOT_KEY_FILE),
        ] {
            let fixture = Fixture::published(label);
            let artifact_path = fixture.generation_path().join(artifact);
            fs::remove_file(&artifact_path).unwrap();
            symlink(
                fixture.generation_path().join(SNAPSHOT_MANIFEST_FILE),
                &artifact_path,
            )
            .unwrap();
            assert!(fixture.inspect().is_err());
        }
    }

    #[test]
    fn pinned_file_rejects_descriptor_path_replacement() {
        let fixture = Fixture::published("descriptor-mismatch");
        let manifest = fixture.generation_path().join(SNAPSHOT_MANIFEST_FILE);
        let pinned = PinnedOwnerOnlyFile::acquire(&manifest, Some(MAX_MANIFEST_BYTES)).unwrap();
        fs::rename(
            &manifest,
            fixture.generation_path().join("displaced-manifest"),
        )
        .unwrap();
        super::super::write_private_file(&manifest, b"{}").unwrap();

        assert!(pinned.validate_current().is_err());
    }
}
