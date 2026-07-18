use std::collections::BTreeMap;

use super::*;

pub(super) const GENERATION_PINS_DIR: &str = "generation-pins";

pub(super) struct SnapshotGenerationReadLease {
    file: File,
    _snapshot_name: String,
}

impl SnapshotGenerationReadLease {
    pub(super) fn acquire(index_root: &Path, snapshot_name: &str) -> Result<Self> {
        let pin_path = generation_pin_path(index_root, snapshot_name);
        let file = open_existing_private_lock(&pin_path)?.ok_or_else(|| {
            FullTextError::internal("full-text published snapshot generation pin missing")
        })?;
        file.lock_shared().map_err(FullTextError::io)?;
        Ok(Self {
            file,
            _snapshot_name: snapshot_name.to_string(),
        })
    }
}

impl Drop for SnapshotGenerationReadLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) struct SnapshotGenerationGcPin {
    pin_path: PathBuf,
    file: File,
}

impl SnapshotGenerationGcPin {
    pub(super) fn try_acquire(pin_path: PathBuf) -> Result<Option<Self>> {
        let file = open_existing_private_lock(&pin_path)?.ok_or_else(|| {
            FullTextError::internal("full-text snapshot generation pin disappeared")
        })?;
        if !try_exclusive_file_lock(&file)? {
            return Ok(None);
        }
        Ok(Some(Self { pin_path, file }))
    }

    pub(super) fn pin_path(&self) -> &Path {
        &self.pin_path
    }
}

impl Drop for SnapshotGenerationGcPin {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) fn create_generation_pin(index_root: &Path, snapshot_name: &str) -> Result<()> {
    validate_snapshot_name(snapshot_name)?;
    let pins_root = index_root.join(GENERATION_PINS_DIR);
    validate_snapshot_directory(&pins_root)?;
    let pin_path = generation_pin_path(index_root, snapshot_name);
    match fs::symlink_metadata(&pin_path) {
        Ok(_) => {
            return Err(FullTextError::internal(
                "full-text snapshot generation pin already exists",
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(FullTextError::io(error)),
    }
    drop(open_or_create_private_lock(&pin_path)?);
    Ok(())
}

pub(super) fn remove_generation_pin(index_root: &Path, snapshot_name: &str) -> Result<()> {
    let pin_path = generation_pin_path(index_root, snapshot_name);
    let metadata = fs::symlink_metadata(&pin_path).map_err(FullTextError::io)?;
    validate_private_lock_metadata(&metadata)?;
    fs::remove_file(pin_path).map_err(FullTextError::io)?;
    sync_directory(&index_root.join(GENERATION_PINS_DIR))
}

pub(super) fn collect_generation_pins(index_root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let pins_root = index_root.join(GENERATION_PINS_DIR);
    validate_snapshot_directory(&pins_root)?;
    let mut pins = BTreeMap::new();
    for entry in fs::read_dir(&pins_root).map_err(FullTextError::io)? {
        let entry = entry.map_err(FullTextError::io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FullTextError::internal("full-text generation pin name invalid"))?;
        let snapshot_name = name
            .strip_suffix(".lock")
            .ok_or_else(|| FullTextError::internal("full-text generation pin name invalid"))?;
        validate_snapshot_name(snapshot_name)?;
        drop(
            open_existing_private_lock(&entry.path())?
                .ok_or_else(|| FullTextError::internal("full-text generation pin disappeared"))?,
        );
        if pins
            .insert(snapshot_name.to_string(), entry.path())
            .is_some()
        {
            return Err(FullTextError::internal(
                "full-text duplicate generation pin",
            ));
        }
    }
    Ok(pins)
}

fn generation_pin_path(index_root: &Path, snapshot_name: &str) -> PathBuf {
    index_root
        .join(GENERATION_PINS_DIR)
        .join(format!("{snapshot_name}.lock"))
}
