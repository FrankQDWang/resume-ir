use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::model::VectorIndexError;
use crate::private_storage::PinnedPrivateDirectory;
use crate::store::{open_lock_file, require_regular_directory, validate_generation};

const STAGING_SUFFIX_HEX_LEN: usize = 24;

pub(super) struct GenerationCandidates {
    pub(super) published: BTreeSet<String>,
    pub(super) reclaim: Vec<(String, PinnedPrivateDirectory)>,
}

pub(super) fn collect_generation_candidates(
    snapshots: &Path,
    retained_generations: &BTreeSet<String>,
) -> Result<GenerationCandidates, VectorIndexError> {
    require_regular_directory(snapshots)?;
    let entries = fs::read_dir(snapshots).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            VectorIndexError::GenerationNotFound
        } else {
            VectorIndexError::Storage
        }
    })?;
    let mut published = BTreeSet::new();
    let mut reclaim = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| VectorIndexError::Storage)?;
        let generation = entry
            .file_name()
            .into_string()
            .map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        validate_generation(&generation).map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        let candidate = PinnedPrivateDirectory::acquire(&entry.path())?;
        published.insert(generation.clone());
        if !retained_generations.contains(&generation) {
            reclaim.push((generation, candidate));
        }
    }
    reclaim.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(GenerationCandidates { published, reclaim })
}

pub(super) struct PinnedVectorGcCandidate {
    pub(super) snapshot: Option<PinnedPrivateDirectory>,
    pub(super) pin_path: PathBuf,
    pub(super) pin: File,
}

impl Drop for PinnedVectorGcCandidate {
    fn drop(&mut self) {
        let _ = self.pin.unlock();
    }
}

pub(super) fn collect_generation_pins(
    pins: &Path,
) -> Result<BTreeMap<String, PathBuf>, VectorIndexError> {
    require_regular_directory(pins)?;
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(pins).map_err(|_| VectorIndexError::Storage)? {
        let entry = entry.map_err(|_| VectorIndexError::Storage)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        let generation = name
            .strip_suffix(".lock")
            .ok_or(VectorIndexError::StorageLayoutInvalid)?;
        validate_generation(generation).map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        drop(open_lock_file(&entry.path(), false)?);
        if entries
            .insert(generation.to_string(), entry.path())
            .is_some()
        {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
    }
    Ok(entries)
}

pub(super) fn collect_staging_candidates(
    staging: &Path,
) -> Result<Vec<PinnedPrivateDirectory>, VectorIndexError> {
    require_regular_directory(staging)?;
    let entries = fs::read_dir(staging).map_err(|_| VectorIndexError::Storage)?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| VectorIndexError::Storage)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        validate_staging_name(&name)?;
        candidates.push(PinnedPrivateDirectory::acquire(&entry.path())?);
    }
    candidates.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(candidates)
}

fn validate_staging_name(name: &str) -> Result<(), VectorIndexError> {
    let (generation, suffix) = name
        .rsplit_once(".tmp-")
        .ok_or(VectorIndexError::StorageLayoutInvalid)?;
    validate_generation(generation).map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
    if suffix.len() == STAGING_SUFFIX_HEX_LEN
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(VectorIndexError::StorageLayoutInvalid)
    }
}
