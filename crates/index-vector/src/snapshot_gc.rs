use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::PathBuf;

use fs4::fs_std::FileExt;

use crate::model::VectorIndexError;
use crate::private_storage::{sync_directory, PinnedPrivateDirectory};
use crate::snapshot_gc_candidates::{
    collect_generation_candidates, collect_generation_pins, collect_staging_candidates,
    PinnedVectorGcCandidate,
};
use crate::snapshot_root::VectorSnapshotRoot;
use crate::store::{
    open_lock_file, validate_generation, GENERATION_PINS_DIR, PUBLICATION_LOCK_FILE,
    READER_LOCK_FILE, SNAPSHOTS_DIR, STAGING_DIR,
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
    commit_snapshot_gc_with_observer(prepared, |_| Ok(()))
}

fn commit_snapshot_gc_with_observer(
    prepared: Box<PreparedVectorSnapshotGc>,
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
    if progress.removed_staging != 0 && sync_directory(&prepared.root.join(STAGING_DIR)).is_err() {
        return partial_report(
            progress,
            total_generations,
            total_staging,
            total_pins,
            VectorSnapshotGcFailurePhase::StagingDurability,
            VectorIndexError::Storage,
        );
    }

    let pin_paths = prepared
        .generation_candidates
        .iter()
        .map(|candidate| candidate.pin_path.clone())
        .collect::<Vec<_>>();
    drop(prepared.generation_candidates);
    for pin_path in pin_paths {
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
    if progress.removed_generation_pins != 0
        && sync_directory(&prepared.root.join(GENERATION_PINS_DIR)).is_err()
    {
        return partial_report(
            progress,
            total_generations,
            total_staging,
            total_pins,
            VectorSnapshotGcFailurePhase::PinDurability,
            VectorIndexError::Storage,
        );
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
            _ => Self::StorageUnavailable,
        }
    }
}

#[cfg(all(test, unix))]
#[path = "snapshot_gc_tests.rs"]
mod tests;
