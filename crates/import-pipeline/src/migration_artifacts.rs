use std::fs;
#[cfg(unix)]
use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{self, ErrorKind};
use std::path::Path;

use meta_store::{OwnedMetaStore, SearchProjectionServiceState, SearchRepairReason, UnixTimestamp};

use super::{ImportPipelineError, PipelineRunControl, Result};

const LEGACY_ARTIFACT_ROOTS: [(&str, &str); 2] = [
    ("search-index", ".search-index.v26-retired"),
    ("vector-index", ".vector-index.v26-retired"),
];

#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationArtifactRetirement {
    pub fulltext_root_retired: bool,
    pub vector_root_retired: bool,
}

impl MigrationArtifactRetirement {
    pub fn retired_any(self) -> bool {
        self.fulltext_root_retired || self.vector_root_retired
    }
}

/// Retires untrusted pre-v27 derived search artifacts before the first v27
/// publication. Source identity and metadata remain owned by the metadata
/// migration; this operation touches only fixed, reproducible index roots.
pub fn prepare_migration_rebuild_artifacts(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    control: &PipelineRunControl,
) -> Result<MigrationArtifactRetirement> {
    control.ensure_running()?;
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if !eligible_for_hard_cut(&state) {
        return Ok(MigrationArtifactRetirement::default());
    }

    let publication_session = match store.try_acquire_search_publication_session() {
        Ok(session) => session,
        Err(error)
            if error.class() == meta_store::MetaStoreErrorClass::MigrationOwnershipRequired =>
        {
            return Err(ImportPipelineError::store(error));
        }
        Err(_) => {
            let _ = store.block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now);
            return Err(ImportPipelineError::artifact_retirement());
        }
    };
    control.ensure_running()?;
    let data_dir = publication_session.canonical_data_dir();
    if validate_data_directory(data_dir).is_err() {
        let _ = store.block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now);
        return Err(ImportPipelineError::artifact_retirement());
    }
    let state = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    if !eligible_for_hard_cut(&state) {
        return Ok(MigrationArtifactRetirement::default());
    }

    match retire_fixed_roots(data_dir, control) {
        Ok(summary) => Ok(summary),
        Err(error) if error.class() == super::ImportPipelineErrorClass::Interrupted => Err(error),
        Err(_) => {
            let _ = store.block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now);
            Err(ImportPipelineError::artifact_retirement())
        }
    }
}

fn eligible_for_hard_cut(state: &meta_store::SearchProjectionState) -> bool {
    state.generation.is_none()
        && state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::MigrationRebuild)
}

fn retire_fixed_roots(
    data_dir: &Path,
    control: &PipelineRunControl,
) -> Result<MigrationArtifactRetirement> {
    validate_data_directory(data_dir).map_err(|_| ImportPipelineError::artifact_retirement())?;
    let mut summary = MigrationArtifactRetirement::default();
    for (index, (root_name, quarantine_name)) in LEGACY_ARTIFACT_ROOTS.iter().enumerate() {
        control.ensure_running()?;
        let root = data_dir.join(root_name);
        let quarantine = data_dir.join(quarantine_name);
        remove_validated_directory_if_present(&quarantine)
            .map_err(|_| ImportPipelineError::artifact_retirement())?;
        let Some(metadata) = validated_directory_metadata(&root)
            .map_err(|_| ImportPipelineError::artifact_retirement())?
        else {
            continue;
        };
        restrict_owner_directory(&root, &metadata)
            .map_err(|_| ImportPipelineError::artifact_retirement())?;
        fs::rename(&root, &quarantine).map_err(|_| ImportPipelineError::artifact_retirement())?;
        sync_directory(data_dir).map_err(|_| ImportPipelineError::artifact_retirement())?;
        control.ensure_running()?;
        remove_validated_directory_if_present(&quarantine)
            .map_err(|_| ImportPipelineError::artifact_retirement())?;
        sync_directory(data_dir).map_err(|_| ImportPipelineError::artifact_retirement())?;
        if index == 0 {
            summary.fulltext_root_retired = true;
        } else {
            summary.vector_root_retired = true;
        }
    }
    control.ensure_running()?;
    Ok(summary)
}

fn validate_data_directory(data_dir: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(data_dir)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "migration data directory must be a non-symlink directory",
        ));
    }
    Ok(())
}

fn validated_directory_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata))
        }
        Ok(_) => Err(io::Error::other(
            "migration artifact path must be a non-symlink directory",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn remove_validated_directory_if_present(path: &Path) -> io::Result<()> {
    if validated_directory_metadata(path)?.is_some() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_owner_directory(path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_owner_directory(_path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH)
        .open(path)?;
    directory.sync_all()
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Err(io::Error::other(
        "migration artifact durability is unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use meta_store::{
        DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, OwnedMetaStore,
        SearchProjectionServiceState, SearchProjectionState, SearchRepairReason, UnixTimestamp,
    };

    use super::{eligible_for_hard_cut, prepare_migration_rebuild_artifacts};

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    fn create_test_store(data_dir: &std::path::Path) -> OwnedMetaStore {
        let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
        };
        let store = owner.open_store().unwrap();
        store.run_migrations().unwrap();
        store
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "resume-ir-v27-artifact-retirement-{}-{suffix}-{}",
                std::process::id(),
                NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migration_rebuild_with_inherited_visible_epoch_is_eligible_for_hard_cut() {
        let state = SearchProjectionState {
            service_state: SearchProjectionServiceState::Repairing,
            generation: None,
            visible_epoch: 9,
            repair_reason: Some(SearchRepairReason::MigrationRebuild),
            publication: None,
            updated_at: UnixTimestamp::from_unix_seconds(1),
        };

        assert!(eligible_for_hard_cut(&state));
    }

    #[test]
    fn migration_rebuild_retires_both_legacy_index_roots() {
        let temp = TestDir::new();
        let store = create_test_store(&temp.0);
        seed_legacy_root(&temp.0.join("search-index"), "fulltext.snapshot.key-v1");
        seed_legacy_root(&temp.0.join("vector-index"), "vector.snapshot.key-v1");

        let summary = prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(2),
            &crate::PipelineRunControl::default(),
        )
        .unwrap();

        assert!(summary.fulltext_root_retired);
        assert!(summary.vector_root_retired);
        assert!(!temp.0.join("search-index").exists());
        assert!(!temp.0.join("vector-index").exists());
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::Repairing
        );
    }

    #[test]
    fn competing_migration_maintenance_fails_fast_without_blocking_repair() {
        let temp = TestDir::new();
        let store = create_test_store(&temp.0);
        let holder_store = store.open_sibling().unwrap();
        let (acquired_sender, acquired_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let holder = thread::spawn(move || {
            let _publication_session = holder_store.wait_for_search_publication_session().unwrap();
            acquired_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        acquired_receiver.recv().unwrap();

        let error = prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(2),
            &crate::PipelineRunControl::default(),
        )
        .unwrap_err();

        assert_eq!(
            error.metadata_class_label(),
            Some("migration_ownership_required")
        );
        let state = store.search_projection_state().unwrap();
        assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        release_sender.send(()).unwrap();
        holder.join().unwrap();
    }

    #[test]
    fn migration_rebuild_resumes_after_interruption_between_artifact_roots() {
        let temp = TestDir::new();
        let store = create_test_store(&temp.0);
        seed_legacy_root(
            &temp.0.join(".search-index.v26-retired"),
            "fulltext.snapshot.key-v1",
        );
        seed_legacy_root(&temp.0.join("vector-index"), "vector.snapshot.key-v1");

        let summary = prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(2),
            &crate::PipelineRunControl::default(),
        )
        .unwrap();

        assert!(!summary.fulltext_root_retired);
        assert!(summary.vector_root_retired);
        assert!(!temp.0.join(".search-index.v26-retired").exists());
        assert!(!temp.0.join("vector-index").exists());
        let retry = prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(3),
            &crate::PipelineRunControl::default(),
        )
        .unwrap();
        assert_eq!(retry, super::MigrationArtifactRetirement::default());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_legacy_artifact_path_blocks_repair_without_following_symlink() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new();
        let store = create_test_store(&temp.0);
        let outside = temp.0.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, temp.0.join("search-index")).unwrap();

        assert!(prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(2),
            &crate::PipelineRunControl::default(),
        )
        .is_err());
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
        assert!(outside.exists());
    }

    #[test]
    fn invalid_publication_lock_layout_blocks_repair() {
        let temp = TestDir::new();
        let store = create_test_store(&temp.0);
        fs::create_dir(temp.0.join("search-publication.lock")).unwrap();

        assert!(prepare_migration_rebuild_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(2),
            &crate::PipelineRunControl::default(),
        )
        .is_err());
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
    }

    fn seed_legacy_root(root: &std::path::Path, marker: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join(marker), b"synthetic legacy marker").unwrap();
    }
}
