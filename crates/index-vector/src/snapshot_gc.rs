use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

use crate::model::VectorIndexError;
use crate::private_storage::{same_open_file_identity, sync_directory, PinnedPrivateDirectory};
use crate::snapshot_gc_candidates::{
    collect_generation_candidates, collect_generation_pins, collect_staging_candidates,
    staging_generation, PinnedVectorGcCandidate,
};
use crate::snapshot_root::VectorSnapshotRoot;
use crate::store::{
    generation_pin_path, open_lock_file, validate_generation, validate_private_lock_metadata,
    GENERATION_PINS_DIR, PUBLICATION_LOCK_FILE, READER_LOCK_FILE, SNAPSHOTS_DIR, STAGING_DIR,
};

/// Global fences acquired before metadata chooses the retained generation set.
pub struct VectorSnapshotGcAcquisition {
    root: PathBuf,
    root_identity: PinnedPrivateDirectory,
    snapshots_identity: PinnedPrivateDirectory,
    staging_identity: PinnedPrivateDirectory,
    pins_identity: PinnedPrivateDirectory,
    root_acquisition_lock: File,
    publication_lock: File,
}

impl VectorSnapshotGcAcquisition {
    fn validate_for(&self, owner: &VectorSnapshotRoot) -> Result<(), VectorIndexError> {
        if self.root != owner.root {
            return Err(VectorIndexError::LeaseRootMismatch);
        }
        owner.root_identity.validate_current()?;
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.staging_identity.validate_current()?;
        self.pins_identity.validate_current()?;
        if !owner.root_identity.same_identity(&self.root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        Ok(())
    }
}

/// Outcome of one non-waiting exact-generation retirement attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorGenerationRetirement {
    /// No snapshot, generation-scoped staging directory, or generation pin was
    /// present for the target.
    Absent,
    /// A publication/root-acquisition fence or exact generation reader is
    /// currently held. No filesystem mutation occurred.
    Deferred,
    /// Every artifact belonging to the exact target was durably removed.
    Retired(VectorGenerationRetirementSummary),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VectorGenerationRetirementSummary {
    removed_generation: bool,
    removed_staging: usize,
    removed_generation_pin: bool,
}

impl VectorGenerationRetirementSummary {
    pub fn removed_generation(self) -> bool {
        self.removed_generation
    }

    pub fn removed_staging(self) -> usize {
        self.removed_staging
    }

    pub fn removed_generation_pin(self) -> bool {
        self.removed_generation_pin
    }
}

/// Retires only `generation` and its generation-scoped staging/pin artifacts.
///
/// The supplied metadata retention set is checked before acquisition. Locking
/// follows the normal vector GC order and never waits; every target identity
/// and generation pin is held before the root reader fence is released.
pub fn try_retire_unpublished_generation(
    root: &Path,
    generation: &str,
    retained_generations: &BTreeSet<String>,
) -> Result<VectorGenerationRetirement, VectorIndexError> {
    validate_generation(generation)?;
    if retained_generations.contains(generation) {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    match fs::symlink_metadata(root) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(VectorGenerationRetirement::Absent);
        }
        Err(_) => return Err(VectorIndexError::Storage),
    }
    if exact_retirement_layout_is_absent(root)? {
        return Ok(VectorGenerationRetirement::Absent);
    }
    let owner = VectorSnapshotRoot::new(root)?;
    let Some(acquisition) = owner.try_acquire_snapshot_gc()? else {
        return Ok(VectorGenerationRetirement::Deferred);
    };
    acquisition.validate_for(&owner)?;

    let generation_path = acquisition.root.join(SNAPSHOTS_DIR).join(generation);
    let snapshot = acquire_optional_generation(&generation_path)?;
    let staging =
        collect_exact_staging_candidates(&acquisition.root.join(STAGING_DIR), generation)?;
    let pin_path = generation_pin_path(&acquisition.root, generation);
    let generation_pin = match fs::symlink_metadata(&pin_path) {
        Ok(_) => {
            let pin = open_lock_file(&pin_path, false)?;
            match pin.try_lock_exclusive() {
                Ok(true) => Some(pin),
                Ok(false) => return Ok(VectorGenerationRetirement::Deferred),
                Err(_) => return Err(VectorIndexError::Storage),
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(_) => return Err(VectorIndexError::Storage),
    };
    if snapshot.is_some() && generation_pin.is_none() {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    if snapshot.is_none() && staging.is_empty() && generation_pin.is_none() {
        return Ok(VectorGenerationRetirement::Absent);
    }

    acquisition.validate_for(&owner)?;
    if let Some(snapshot) = &snapshot {
        snapshot.validate_current()?;
    }
    for candidate in &staging {
        candidate.validate_current()?;
    }
    if let Some(pin) = &generation_pin {
        validate_generation_pin(&pin_path, pin)?;
    }

    let VectorSnapshotGcAcquisition {
        root,
        root_identity,
        snapshots_identity,
        staging_identity,
        pins_identity,
        root_acquisition_lock,
        publication_lock: _publication_lock,
    } = acquisition;
    drop(root_acquisition_lock);
    root_identity.validate_current()?;
    snapshots_identity.validate_current()?;
    staging_identity.validate_current()?;
    pins_identity.validate_current()?;

    let mut summary = VectorGenerationRetirementSummary::default();
    if let Some(snapshot) = &snapshot {
        snapshot.validate_current()?;
        fs::remove_dir_all(snapshot.path()).map_err(|_| VectorIndexError::Storage)?;
        sync_directory(&root.join(SNAPSHOTS_DIR))?;
        summary.removed_generation = true;
    }
    for candidate in &staging {
        candidate.validate_current()?;
        fs::remove_dir_all(candidate.path()).map_err(|_| VectorIndexError::Storage)?;
        summary.removed_staging += 1;
    }
    if !staging.is_empty() {
        sync_directory(&root.join(STAGING_DIR))?;
    }
    if let Some(pin) = generation_pin {
        validate_generation_pin(&pin_path, &pin)?;
        drop(pin);
        fs::remove_file(&pin_path).map_err(|_| VectorIndexError::Storage)?;
        sync_directory(&root.join(GENERATION_PINS_DIR))?;
        summary.removed_generation_pin = true;
    }

    root_identity.validate_current()?;
    snapshots_identity.validate_current()?;
    staging_identity.validate_current()?;
    pins_identity.validate_current()?;
    Ok(VectorGenerationRetirement::Retired(summary))
}

fn exact_retirement_layout_is_absent(root: &Path) -> Result<bool, VectorIndexError> {
    let controlled = [
        SNAPSHOTS_DIR,
        STAGING_DIR,
        GENERATION_PINS_DIR,
        READER_LOCK_FILE,
        PUBLICATION_LOCK_FILE,
    ];
    let mut present = 0_usize;
    for entry in &controlled {
        match fs::symlink_metadata(root.join(entry)) {
            Ok(_) => present += 1,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(VectorIndexError::Storage),
        }
    }
    if present == 0 {
        let mut entries = fs::read_dir(root).map_err(|_| VectorIndexError::Storage)?;
        return entries
            .next()
            .transpose()
            .map(|entry| entry.is_none())
            .map_err(|_| VectorIndexError::Storage);
    }
    if present != controlled.len() {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(false)
}

fn acquire_optional_generation(
    path: &Path,
) -> Result<Option<PinnedPrivateDirectory>, VectorIndexError> {
    match fs::symlink_metadata(path) {
        Ok(_) => PinnedPrivateDirectory::acquire(path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(VectorIndexError::Storage),
    }
}

fn collect_exact_staging_candidates(
    root: &Path,
    generation: &str,
) -> Result<Vec<PinnedPrivateDirectory>, VectorIndexError> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| VectorIndexError::Storage)? {
        let entry = entry.map_err(|_| VectorIndexError::Storage)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| VectorIndexError::StorageLayoutInvalid)?;
        if staging_generation(&name)? == generation {
            candidates.push(PinnedPrivateDirectory::acquire(&entry.path())?);
        }
    }
    candidates.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(candidates)
}

fn validate_generation_pin(path: &Path, pin: &File) -> Result<(), VectorIndexError> {
    let opened = pin.metadata().map_err(|_| VectorIndexError::Storage)?;
    validate_private_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path).map_err(|_| VectorIndexError::Storage)?;
    validate_private_lock_metadata(&current)?;
    if same_open_file_identity(pin, path, &opened, &current)? {
        Ok(())
    } else {
        Err(VectorIndexError::StorageLayoutInvalid)
    }
}

pub enum VectorSnapshotGcPreparation {
    /// At least one candidate generation has a live reader. No mutation has
    /// occurred and the attempt retains no locks.
    Deferred,
    /// Every candidate identity and generation pin is held for commit.
    Prepared(Box<PreparedVectorSnapshotGc>),
}

/// Prepared reclamation plan. The root-acquisition fence has already been
/// released, allowing retained readers to enter during physical deletion.
pub struct PreparedVectorSnapshotGc {
    root: PathBuf,
    root_identity: PinnedPrivateDirectory,
    snapshots_identity: PinnedPrivateDirectory,
    staging_identity: PinnedPrivateDirectory,
    pins_identity: PinnedPrivateDirectory,
    _publication_lock: File,
    generation_candidates: Vec<PinnedVectorGcCandidate>,
    staging_candidates: Vec<PinnedPrivateDirectory>,
}

impl PreparedVectorSnapshotGc {
    fn validate_layout(&self) -> Result<(), VectorIndexError> {
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.staging_identity.validate_current()?;
        self.pins_identity.validate_current()
    }
}

impl VectorSnapshotRoot {
    /// Attempts to acquire root-acquisition then publication fences without
    /// waiting. Upper layers acquire full-text first, then vector.
    pub fn try_acquire_snapshot_gc(
        &self,
    ) -> Result<Option<VectorSnapshotGcAcquisition>, VectorIndexError> {
        self.root_identity.validate_current()?;
        let root_identity = PinnedPrivateDirectory::acquire(&self.root)?;
        if !self.root_identity.same_identity(&root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        let snapshots_identity = PinnedPrivateDirectory::acquire(&self.root.join(SNAPSHOTS_DIR))?;
        let staging_identity = PinnedPrivateDirectory::acquire(&self.root.join(STAGING_DIR))?;
        let pins_identity = PinnedPrivateDirectory::acquire(&self.root.join(GENERATION_PINS_DIR))?;
        let root_acquisition_lock = open_lock_file(&self.root.join(READER_LOCK_FILE), false)?;
        match root_acquisition_lock.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(_) => return Err(VectorIndexError::Storage),
        }
        let publication_lock = open_lock_file(&self.root.join(PUBLICATION_LOCK_FILE), false)?;
        match publication_lock.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(_) => return Err(VectorIndexError::Storage),
        }
        let acquisition = VectorSnapshotGcAcquisition {
            root: self.root.clone(),
            root_identity,
            snapshots_identity,
            staging_identity,
            pins_identity,
            root_acquisition_lock,
            publication_lock,
        };
        acquisition.validate_for(self)?;
        Ok(Some(acquisition))
    }

    /// Pins every candidate before releasing the root-acquisition fence. Any
    /// busy candidate returns `Deferred` with zero filesystem mutations.
    pub fn prepare_snapshot_gc(
        &self,
        acquisition: VectorSnapshotGcAcquisition,
        retained_generations: &BTreeSet<String>,
    ) -> Result<VectorSnapshotGcPreparation, VectorIndexError> {
        acquisition.validate_for(self)?;
        for generation in retained_generations {
            validate_generation(generation)?;
        }
        let snapshots = self.root.join(SNAPSHOTS_DIR);
        let staging = self.root.join(STAGING_DIR);
        let pins = self.root.join(GENERATION_PINS_DIR);
        let generation_candidates =
            collect_generation_candidates(&snapshots, retained_generations)?;
        let staging_candidates = collect_staging_candidates(&staging)?;
        let pin_entries = collect_generation_pins(&pins)?;
        if generation_candidates
            .published
            .iter()
            .any(|generation| !pin_entries.contains_key(generation))
        {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }

        let mut pinned_candidates = Vec::new();
        for (generation, snapshot) in generation_candidates.reclaim {
            let pin_path = pin_entries
                .get(&generation)
                .ok_or(VectorIndexError::StorageLayoutInvalid)?;
            let pin = open_lock_file(pin_path, false)?;
            match pin.try_lock_exclusive() {
                Ok(true) => {}
                Ok(false) => return Ok(VectorSnapshotGcPreparation::Deferred),
                Err(_) => return Err(VectorIndexError::Storage),
            }
            pinned_candidates.push(PinnedVectorGcCandidate {
                snapshot: Some(snapshot),
                pin_path: pin_path.clone(),
                pin,
            });
        }
        for (generation, pin_path) in &pin_entries {
            if !generation_candidates.published.contains(generation) {
                let pin = open_lock_file(pin_path, false)?;
                match pin.try_lock_exclusive() {
                    Ok(true) => {}
                    Ok(false) => return Ok(VectorSnapshotGcPreparation::Deferred),
                    Err(_) => return Err(VectorIndexError::Storage),
                }
                pinned_candidates.push(PinnedVectorGcCandidate {
                    snapshot: None,
                    pin_path: pin_path.clone(),
                    pin,
                });
            }
        }
        acquisition.validate_for(self)?;
        validate_candidate_identities(&pinned_candidates, &staging_candidates)?;

        let VectorSnapshotGcAcquisition {
            root,
            root_identity,
            snapshots_identity,
            staging_identity,
            pins_identity,
            root_acquisition_lock,
            publication_lock,
        } = acquisition;
        drop(root_acquisition_lock);
        Ok(VectorSnapshotGcPreparation::Prepared(Box::new(
            PreparedVectorSnapshotGc {
                root,
                root_identity,
                snapshots_identity,
                staging_identity,
                pins_identity,
                _publication_lock: publication_lock,
                generation_candidates: pinned_candidates,
                staging_candidates,
            },
        )))
    }
}

/// Commits a prepared plan and always reports post-prepare partial progress.
/// Retrying acquire/prepare/commit converges after a partial failure.
pub fn commit_snapshot_gc(prepared: Box<PreparedVectorSnapshotGc>) -> VectorSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, || false, |_| Ok(()))
}

/// Commits a prepared plan while observing cancellation between crash-safe
/// generation, staging-directory, durability, and pin-removal boundaries.
pub fn commit_snapshot_gc_with_cancel_check(
    prepared: Box<PreparedVectorSnapshotGc>,
    cancel_check: &dyn Fn() -> bool,
) -> VectorSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, cancel_check, |_| Ok(()))
}

#[cfg(test)]
fn commit_snapshot_gc_with_observer(
    prepared: Box<PreparedVectorSnapshotGc>,
    after_removal: impl FnMut(usize) -> Result<(), VectorIndexError>,
) -> VectorSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, || false, after_removal)
}

fn commit_snapshot_gc_with_control_and_observer(
    prepared: Box<PreparedVectorSnapshotGc>,
    mut cancel_check: impl FnMut() -> bool,
    mut after_removal: impl FnMut(usize) -> Result<(), VectorIndexError>,
) -> VectorSnapshotGcCommitReport {
    let prepared = *prepared;
    let total_generations = prepared
        .generation_candidates
        .iter()
        .filter(|candidate| candidate.snapshot.is_some())
        .count();
    let total_staging = prepared.staging_candidates.len();
    let total_pins = prepared.generation_candidates.len();
    let mut progress = VectorGcSummary::default();
    let mut removed_entries = 0_usize;

    if cancel_check() {
        return VectorSnapshotGcCommitReport::Interrupted(progress);
    }

    let preflight = prepared.validate_layout().and_then(|_| {
        validate_candidate_identities(
            &prepared.generation_candidates,
            &prepared.staging_candidates,
        )
    });
    if let Err(error) = preflight {
        return partial_report(
            progress,
            total_generations,
            total_staging,
            total_pins,
            VectorSnapshotGcFailurePhase::Preflight,
            error,
        );
    }

    for candidate in &prepared.generation_candidates {
        let Some(snapshot) = &candidate.snapshot else {
            continue;
        };
        if cancel_check() {
            return VectorSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = snapshot.validate_current() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::GenerationRemoval,
                error,
            );
        }
        if fs::remove_dir_all(snapshot.path()).is_err() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::GenerationRemoval,
                VectorIndexError::Storage,
            );
        }
        progress.removed_generations += 1;
        removed_entries += 1;
        if let Err(error) = after_removal(removed_entries) {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::GenerationRemoval,
                error,
            );
        }
    }
    for candidate in &prepared.staging_candidates {
        if cancel_check() {
            return VectorSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = candidate.validate_current() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::StagingRemoval,
                error,
            );
        }
        if fs::remove_dir_all(candidate.path()).is_err() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::StagingRemoval,
                VectorIndexError::Storage,
            );
        }
        progress.removed_staging += 1;
        removed_entries += 1;
        if let Err(error) = after_removal(removed_entries) {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::StagingRemoval,
                error,
            );
        }
    }

    if cancel_check() {
        return VectorSnapshotGcCommitReport::Interrupted(progress);
    }
    if progress.removed_generations != 0
        && sync_directory(&prepared.root.join(SNAPSHOTS_DIR)).is_err()
    {
        return partial_report(
            progress,
            total_generations,
            total_staging,
            total_pins,
            VectorSnapshotGcFailurePhase::GenerationDurability,
            VectorIndexError::Storage,
        );
    }
    if progress.removed_staging != 0 {
        if cancel_check() {
            return VectorSnapshotGcCommitReport::Interrupted(progress);
        }
        if sync_directory(&prepared.root.join(STAGING_DIR)).is_err() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::StagingDurability,
                VectorIndexError::Storage,
            );
        }
    }

    let pin_paths = prepared
        .generation_candidates
        .iter()
        .map(|candidate| candidate.pin_path.clone())
        .collect::<Vec<_>>();
    drop(prepared.generation_candidates);
    for pin_path in pin_paths {
        if cancel_check() {
            return VectorSnapshotGcCommitReport::Interrupted(progress);
        }
        if fs::remove_file(pin_path).is_err() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::PinRemoval,
                VectorIndexError::Storage,
            );
        }
        progress.removed_generation_pins += 1;
    }
    if progress.removed_generation_pins != 0 {
        if cancel_check() {
            return VectorSnapshotGcCommitReport::Interrupted(progress);
        }
        if sync_directory(&prepared.root.join(GENERATION_PINS_DIR)).is_err() {
            return partial_report(
                progress,
                total_generations,
                total_staging,
                total_pins,
                VectorSnapshotGcFailurePhase::PinDurability,
                VectorIndexError::Storage,
            );
        }
    }
    if cancel_check() {
        return VectorSnapshotGcCommitReport::Interrupted(progress);
    }
    let final_layout = prepared
        .root_identity
        .validate_current()
        .and_then(|_| prepared.snapshots_identity.validate_current())
        .and_then(|_| prepared.staging_identity.validate_current())
        .and_then(|_| prepared.pins_identity.validate_current());
    if let Err(error) = final_layout {
        return partial_report(
            progress,
            total_generations,
            total_staging,
            total_pins,
            VectorSnapshotGcFailurePhase::FinalValidation,
            error,
        );
    }
    VectorSnapshotGcCommitReport::Complete(progress)
}

fn validate_candidate_identities(
    generations: &[PinnedVectorGcCandidate],
    staging: &[PinnedPrivateDirectory],
) -> Result<(), VectorIndexError> {
    for candidate in generations {
        if let Some(snapshot) = &candidate.snapshot {
            snapshot.validate_current()?;
        }
    }
    for candidate in staging {
        candidate.validate_current()?;
    }
    Ok(())
}

fn partial_report(
    progress: VectorGcSummary,
    total_generations: usize,
    total_staging: usize,
    total_pins: usize,
    failure_phase: VectorSnapshotGcFailurePhase,
    error: VectorIndexError,
) -> VectorSnapshotGcCommitReport {
    VectorSnapshotGcCommitReport::PartialFailure(VectorGcPartialFailure {
        progress,
        remaining_generations: total_generations.saturating_sub(progress.removed_generations),
        remaining_staging: total_staging.saturating_sub(progress.removed_staging),
        remaining_generation_pins: total_pins.saturating_sub(progress.removed_generation_pins),
        failure_phase,
        failure_class: VectorSnapshotGcFailureClass::from_error(error),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VectorGcSummary {
    removed_generations: usize,
    removed_staging: usize,
    removed_generation_pins: usize,
}

impl VectorGcSummary {
    pub fn removed_generations(self) -> usize {
        self.removed_generations
    }

    pub fn removed_staging(self) -> usize {
        self.removed_staging
    }

    pub fn removed_generation_pins(self) -> usize {
        self.removed_generation_pins
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorSnapshotGcCommitReport {
    Complete(VectorGcSummary),
    Interrupted(VectorGcSummary),
    PartialFailure(VectorGcPartialFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorGcPartialFailure {
    progress: VectorGcSummary,
    remaining_generations: usize,
    remaining_staging: usize,
    remaining_generation_pins: usize,
    failure_phase: VectorSnapshotGcFailurePhase,
    failure_class: VectorSnapshotGcFailureClass,
}

impl VectorGcPartialFailure {
    pub fn progress(self) -> VectorGcSummary {
        self.progress
    }

    pub fn remaining_generations(self) -> usize {
        self.remaining_generations
    }

    pub fn remaining_staging(self) -> usize {
        self.remaining_staging
    }

    pub fn remaining_generation_pins(self) -> usize {
        self.remaining_generation_pins
    }

    pub fn failure_phase(self) -> VectorSnapshotGcFailurePhase {
        self.failure_phase
    }

    pub fn failure_class(self) -> VectorSnapshotGcFailureClass {
        self.failure_class
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorSnapshotGcFailurePhase {
    Preflight,
    GenerationRemoval,
    StagingRemoval,
    GenerationDurability,
    StagingDurability,
    PinRemoval,
    PinDurability,
    FinalValidation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorSnapshotGcFailureClass {
    LayoutChanged,
    StorageUnavailable,
}

impl VectorSnapshotGcFailureClass {
    fn from_error(error: VectorIndexError) -> Self {
        match error {
            VectorIndexError::LeaseRootMismatch | VectorIndexError::StorageLayoutInvalid => {
                Self::LayoutChanged
            }
            VectorIndexError::Cancelled
            | VectorIndexError::PublicationBusy
            | VectorIndexError::InvalidDimension { .. }
            | VectorIndexError::InvalidVectorValue
            | VectorIndexError::InvalidModelId
            | VectorIndexError::InvalidIdentity
            | VectorIndexError::InvalidGeneration
            | VectorIndexError::InvalidModelContract
            | VectorIndexError::SemanticUnavailable
            | VectorIndexError::PublicationProjectionMismatch
            | VectorIndexError::DuplicateVectorId
            | VectorIndexError::ConflictingDocumentVersion
            | VectorIndexError::GenerationAlreadyExists
            | VectorIndexError::GenerationNotFound
            | VectorIndexError::SchemaMismatch
            | VectorIndexError::CorruptSnapshot
            | VectorIndexError::Storage => Self::StorageUnavailable,
        }
    }
}

#[cfg(all(test, unix))]
#[path = "snapshot_gc_tests.rs"]
mod tests;
