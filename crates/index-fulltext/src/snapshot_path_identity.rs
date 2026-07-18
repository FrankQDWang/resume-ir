use super::*;

pub(super) struct PinnedSnapshotDirectory {
    path: PathBuf,
    identity: same_file::Handle,
}

impl PinnedSnapshotDirectory {
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        validate_snapshot_directory(path)?;
        let pinned = Self {
            path: path.to_path_buf(),
            identity: same_file::Handle::from_path(path).map_err(FullTextError::io)?,
        };
        pinned.validate_current()?;
        Ok(pinned)
    }

    pub(super) fn validate_current(&self) -> Result<()> {
        self.validate_identity_at(&self.path)
    }

    pub(super) fn validate_identity_at(&self, path: &Path) -> Result<()> {
        validate_snapshot_directory(path)?;
        let current = same_file::Handle::from_path(path).map_err(FullTextError::io)?;
        validate_snapshot_directory(path)?;
        if self.identity == current {
            Ok(())
        } else {
            Err(FullTextError::internal(
                "full-text snapshot directory identity changed",
            ))
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}
