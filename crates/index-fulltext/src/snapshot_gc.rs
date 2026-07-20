use super::*;
use crate::snapshot_generation::{collect_generation_pins, SnapshotGenerationGcPin};

const STAGING_SUFFIX_HEX_LEN: usize = 16;

/// Acquisition fence for one full-text reclamation attempt. Coordinators
/// acquire this before the vector fence, then read metadata retention.
pub struct FullTextSnapshotGcAcquisition {
    index_root: PathBuf,
    root_acquisition: SnapshotGcLease,
    publication: SnapshotPublicationLease,
}

/// Attempts to acquire root-acquisition then publication fences without
/// waiting. `None` means collection must be deferred and no fence is retained.
pub fn try_acquire_snapshot_gc(index_root: &Path) -> Result<Option<FullTextSnapshotGcAcquisition>> {
    let index_root = fs::canonicalize(index_root).map_err(FullTextError::io)?;
    validate_snapshot_directory(&index_root)?;
    let Some(root_acquisition) = SnapshotGcLease::try_acquire(&index_root)? else {
        return Ok(None);
    };
    let Some(publication) = SnapshotPublicationLease::try_acquire_existing(&index_root)? else {
        return Ok(None);
    };
    if !root_acquisition.same_layout_as(&publication) {
        return Err(FullTextError::internal(
            "full-text GC fences belong to different storage layouts",
        ));
    }
    Ok(Some(FullTextSnapshotGcAcquisition {
        index_root,
        root_acquisition,
        publication,
    }))
}

impl FullTextSnapshotGcAcquisition {
    fn protects(&self, index_root: &Path) -> Result<bool> {
        if self.index_root != fs::canonicalize(index_root).map_err(FullTextError::io)? {
            return Ok(false);
        }
        self.root_acquisition.validate_layout()?;
        self.publication.validate_layout()?;
        Ok(self.root_acquisition.same_layout_as(&self.publication))
    }
}

/// Outcome of one non-waiting retirement attempt for an exact unpublished
/// generation. The caller must supply the metadata-retained generation set;
/// a retained target is rejected before any filesystem mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTextGenerationRetirement {
    /// No snapshot, generation-scoped staging directory, or generation pin was
    /// present for the target.
    Absent,
    /// A publication/root-acquisition fence or exact generation reader is
    /// currently held. No filesystem mutation occurred.
    Deferred,
    /// Every artifact belonging to the exact target was durably removed.
    Retired(FullTextGenerationRetirementSummary),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FullTextGenerationRetirementSummary {
    removed_snapshot: bool,
    removed_staging: usize,
    removed_generation_pin: bool,
}

impl FullTextGenerationRetirementSummary {
    pub fn removed_snapshot(self) -> bool {
        self.removed_snapshot
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
/// Acquisition follows the normal GC lock order and never waits. All target
/// identities and the generation pin are acquired before the root reader fence
/// is released, so a racing exact reader either wins and defers this attempt or
/// observes the generation as absent. Unrelated snapshots, staging directories,
/// and pins are never selected for deletion.
pub fn try_retire_unpublished_generation(
    index_root: &Path,
    generation: &str,
    retained_generations: &BTreeSet<String>,
) -> Result<FullTextGenerationRetirement> {
    validate_snapshot_name(generation)?;
    if retained_generations.contains(generation) {
        return Err(FullTextError::internal(
            "retained full-text generation cannot be retired",
        ));
    }
    match fs::symlink_metadata(index_root) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(FullTextGenerationRetirement::Absent);
        }
        Err(error) => return Err(FullTextError::io(error)),
    }
    let Some(acquisition) = try_acquire_snapshot_gc(index_root)? else {
        return Ok(FullTextGenerationRetirement::Deferred);
    };
    if !acquisition.protects(&acquisition.index_root)? {
        return Err(FullTextError::internal(
            "full-text retirement acquisition belongs to another index root",
        ));
    }

    let snapshot = acquire_optional_snapshot_directory(
        &acquisition.index_root.join(SNAPSHOTS_DIR).join(generation),
    )?;
    let encrypted_staging = collect_exact_staging_candidates(
        &acquisition.index_root.join(SNAPSHOTS_DIR),
        generation,
        ".tmp-",
    )?;
    let staging = collect_exact_staging_candidates(
        &acquisition.index_root.join(STAGING_DIR),
        generation,
        ".staging-",
    )?;
    let generation_pins = collect_generation_pins(&acquisition.index_root)?;
    let generation_pin = generation_pins.get(generation).cloned();
    if snapshot.is_some() && generation_pin.is_none() {
        return Err(FullTextError::internal(
            "full-text published snapshot generation pin missing",
        ));
    }
    let generation_pin = match generation_pin {
        Some(pin_path) => {
            let Some(pin) = SnapshotGenerationGcPin::try_acquire(pin_path)? else {
                return Ok(FullTextGenerationRetirement::Deferred);
            };
            Some(pin)
        }
        None => None,
    };
    if snapshot.is_none()
        && encrypted_staging.is_empty()
        && staging.is_empty()
        && generation_pin.is_none()
    {
        return Ok(FullTextGenerationRetirement::Absent);
    }

    if !acquisition.protects(&acquisition.index_root)? {
        return Err(FullTextError::internal(
            "full-text retirement layout changed during preparation",
        ));
    }
    if let Some(snapshot) = &snapshot {
        snapshot.validate_current()?;
    }
    for candidate in encrypted_staging.iter().chain(&staging) {
        candidate.validate_current()?;
    }
    if let Some(pin) = &generation_pin {
        pin.validate_current()?;
    }

    let FullTextSnapshotGcAcquisition {
        index_root,
        root_acquisition,
        publication,
    } = acquisition;
    drop(root_acquisition);
    publication.validate_layout()?;

    let mut summary = FullTextGenerationRetirementSummary::default();
    if let Some(snapshot) = &snapshot {
        snapshot.validate_current()?;
        remove_pinned_snapshot_dir_all(snapshot)?;
        summary.removed_snapshot = true;
    }
    for candidate in encrypted_staging.iter().chain(&staging) {
        candidate.validate_current()?;
        remove_pinned_snapshot_dir_all(candidate)?;
        summary.removed_staging += 1;
    }
    if summary.removed_snapshot || !encrypted_staging.is_empty() {
        sync_directory(&index_root.join(SNAPSHOTS_DIR))?;
    }
    if !staging.is_empty() {
        sync_directory(&index_root.join(STAGING_DIR))?;
    }

    if let Some(pin) = generation_pin {
        pin.validate_current()?;
        let pin_path = pin.pin_path().to_path_buf();
        drop(pin);
        fs::remove_file(pin_path).map_err(FullTextError::io)?;
        sync_directory(&index_root.join(GENERATION_PINS_DIR))?;
        summary.removed_generation_pin = true;
    }
    publication.validate_layout()?;
    Ok(FullTextGenerationRetirement::Retired(summary))
}

/// Result of preflighting every candidate under the acquisition fence.
pub enum FullTextSnapshotGcPreparation {
    /// At least one candidate generation has a live reader. No deletion has
    /// occurred and no lock remains held by this attempt.
    Deferred,
    /// Every candidate and generation pin is pinned for a safe commit.
    Prepared(Box<PreparedFullTextSnapshotGc>),
}

/// Prepared reclamation plan. It retains the publication fence and candidate
/// pins, but not the root-acquisition fence, so retained readers can enter
/// while physical deletion runs.
pub struct PreparedFullTextSnapshotGc {
    index_root: PathBuf,
    publication: SnapshotPublicationLease,
    snapshots: Vec<(String, PinnedSnapshotDirectory)>,
    encrypted_staging: Vec<PinnedSnapshotDirectory>,
    staging: Vec<PinnedSnapshotDirectory>,
    generation_pins: Vec<SnapshotGenerationGcPin>,
}

/// Acquires every candidate pin and identity before allowing deletion. Any
/// busy candidate returns `Deferred` with zero filesystem mutations.
pub fn prepare_snapshot_gc(
    acquisition: FullTextSnapshotGcAcquisition,
    retained_snapshots: &BTreeSet<String>,
) -> Result<FullTextSnapshotGcPreparation> {
    if !acquisition.protects(&acquisition.index_root)? {
        return Err(FullTextError::internal(
            "full-text GC acquisition belongs to another index root",
        ));
    }
    for snapshot_name in retained_snapshots {
        validate_snapshot_name(snapshot_name)?;
    }

    let snapshots_root = acquisition.index_root.join(SNAPSHOTS_DIR);
    let staging_root = acquisition.index_root.join(STAGING_DIR);
    let candidates = collect_snapshot_candidates(&snapshots_root, retained_snapshots)?;
    let staging_candidates = collect_staging_candidates(&staging_root)?;
    let generation_pins = collect_generation_pins(&acquisition.index_root)?;
    if candidates
        .published
        .iter()
        .any(|snapshot_name| !generation_pins.contains_key(snapshot_name))
    {
        return Err(FullTextError::internal(
            "full-text published snapshot generation pin missing",
        ));
    }

    let mut acquired_pins = Vec::new();
    for (snapshot_name, _) in &candidates.snapshots {
        let pin_path = generation_pins
            .get(snapshot_name)
            .ok_or_else(|| FullTextError::internal("full-text snapshot generation pin missing"))?;
        let Some(pin) = SnapshotGenerationGcPin::try_acquire(pin_path.clone())? else {
            return Ok(FullTextSnapshotGcPreparation::Deferred);
        };
        acquired_pins.push(pin);
    }
    for (snapshot_name, pin_path) in &generation_pins {
        if !candidates.published.contains(snapshot_name) {
            let Some(pin) = SnapshotGenerationGcPin::try_acquire(pin_path.clone())? else {
                return Ok(FullTextSnapshotGcPreparation::Deferred);
            };
            acquired_pins.push(pin);
        }
    }

    if !acquisition.protects(&acquisition.index_root)? {
        return Err(FullTextError::internal(
            "full-text GC storage layout changed during preparation",
        ));
    }
    for (_, candidate) in &candidates.snapshots {
        candidate.validate_current()?;
    }
    for candidate in candidates
        .encrypted_staging
        .iter()
        .chain(&staging_candidates)
    {
        candidate.validate_current()?;
    }

    let FullTextSnapshotGcAcquisition {
        index_root,
        root_acquisition,
        publication,
    } = acquisition;
    drop(root_acquisition);
    Ok(FullTextSnapshotGcPreparation::Prepared(Box::new(
        PreparedFullTextSnapshotGc {
            index_root,
            publication,
            snapshots: candidates.snapshots,
            encrypted_staging: candidates.encrypted_staging,
            staging: staging_candidates,
            generation_pins: acquired_pins,
        },
    )))
}

/// Commits a prepared plan. All post-prepare failures are encoded in the
/// bounded report so a partially deleted generation set is never hidden by a
/// plain `Result` error. A later prepare/commit attempt converges idempotently.
pub fn commit_snapshot_gc(
    prepared: Box<PreparedFullTextSnapshotGc>,
) -> FullTextSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, || false, |_| Ok(()))
}

/// Commits a prepared plan while observing cancellation between crash-safe
/// generation, staging-directory, durability, and pin-removal boundaries.
pub fn commit_snapshot_gc_with_cancel_check(
    prepared: Box<PreparedFullTextSnapshotGc>,
    cancel_check: &dyn Fn() -> bool,
) -> FullTextSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, cancel_check, |_| Ok(()))
}

#[cfg(test)]
fn commit_snapshot_gc_with_observer(
    prepared: Box<PreparedFullTextSnapshotGc>,
    after_removal: impl FnMut(usize) -> Result<()>,
) -> FullTextSnapshotGcCommitReport {
    commit_snapshot_gc_with_control_and_observer(prepared, || false, after_removal)
}

fn commit_snapshot_gc_with_control_and_observer(
    prepared: Box<PreparedFullTextSnapshotGc>,
    mut cancel_check: impl FnMut() -> bool,
    mut after_removal: impl FnMut(usize) -> Result<()>,
) -> FullTextSnapshotGcCommitReport {
    let prepared = *prepared;
    let total_snapshots = prepared.snapshots.len();
    let total_staging = prepared.encrypted_staging.len() + prepared.staging.len();
    let total_pins = prepared.generation_pins.len();
    let mut progress = SnapshotPurgeSummary::default();
    let mut removed_entries = 0_usize;

    if cancel_check() {
        return FullTextSnapshotGcCommitReport::Interrupted(progress);
    }

    let preflight = prepared
        .publication
        .validate_layout()
        .and_then(|_| validate_prepared_candidates(&prepared));
    if let Err(error) = preflight {
        return partial_report(
            progress,
            total_snapshots,
            total_staging,
            total_pins,
            FullTextSnapshotGcFailurePhase::Preflight,
            &error,
        );
    }

    for (_, candidate) in &prepared.snapshots {
        if cancel_check() {
            return FullTextSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = candidate
            .validate_current()
            .and_then(|_| remove_pinned_snapshot_dir_all(candidate))
        {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::SnapshotRemoval,
                &error,
            );
        }
        progress.removed_snapshots += 1;
        removed_entries += 1;
        if let Err(error) = after_removal(removed_entries) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::SnapshotRemoval,
                &error,
            );
        }
    }
    for candidate in prepared.encrypted_staging.iter().chain(&prepared.staging) {
        if cancel_check() {
            return FullTextSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = candidate
            .validate_current()
            .and_then(|_| remove_pinned_snapshot_dir_all(candidate))
        {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::StagingRemoval,
                &error,
            );
        }
        progress.removed_staging += 1;
        removed_entries += 1;
        if let Err(error) = after_removal(removed_entries) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::StagingRemoval,
                &error,
            );
        }
    }

    if cancel_check() {
        return FullTextSnapshotGcCommitReport::Interrupted(progress);
    }
    let snapshots_root = prepared.index_root.join(SNAPSHOTS_DIR);
    let staging_root = prepared.index_root.join(STAGING_DIR);
    if progress.removed_snapshots != 0 || !prepared.encrypted_staging.is_empty() {
        if let Err(error) = sync_directory(&snapshots_root) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::SnapshotDurability,
                &error,
            );
        }
    }
    if !prepared.staging.is_empty() {
        if cancel_check() {
            return FullTextSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = sync_directory(&staging_root) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::StagingDurability,
                &error,
            );
        }
    }

    let pin_paths = prepared
        .generation_pins
        .iter()
        .map(|pin| pin.pin_path().to_path_buf())
        .collect::<Vec<_>>();
    drop(prepared.generation_pins);
    for pin_path in pin_paths {
        if cancel_check() {
            return FullTextSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = fs::remove_file(pin_path).map_err(FullTextError::io) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::PinRemoval,
                &error,
            );
        }
        progress.removed_generation_pins += 1;
    }
    if progress.removed_generation_pins != 0 {
        if cancel_check() {
            return FullTextSnapshotGcCommitReport::Interrupted(progress);
        }
        if let Err(error) = sync_directory(&prepared.index_root.join(GENERATION_PINS_DIR)) {
            return partial_report(
                progress,
                total_snapshots,
                total_staging,
                total_pins,
                FullTextSnapshotGcFailurePhase::PinDurability,
                &error,
            );
        }
    }
    if cancel_check() {
        return FullTextSnapshotGcCommitReport::Interrupted(progress);
    }
    if let Err(error) = prepared.publication.validate_layout() {
        return partial_report(
            progress,
            total_snapshots,
            total_staging,
            total_pins,
            FullTextSnapshotGcFailurePhase::FinalValidation,
            &error,
        );
    }
    FullTextSnapshotGcCommitReport::Complete(progress)
}

fn validate_prepared_candidates(prepared: &PreparedFullTextSnapshotGc) -> Result<()> {
    for (_, candidate) in &prepared.snapshots {
        candidate.validate_current()?;
    }
    for candidate in prepared.encrypted_staging.iter().chain(&prepared.staging) {
        candidate.validate_current()?;
    }
    Ok(())
}

fn partial_report(
    progress: SnapshotPurgeSummary,
    total_snapshots: usize,
    total_staging: usize,
    total_pins: usize,
    failure_phase: FullTextSnapshotGcFailurePhase,
    error: &FullTextError,
) -> FullTextSnapshotGcCommitReport {
    FullTextSnapshotGcCommitReport::PartialFailure(SnapshotPurgePartialFailure {
        progress,
        remaining_snapshots: total_snapshots.saturating_sub(progress.removed_snapshots),
        remaining_staging: total_staging.saturating_sub(progress.removed_staging),
        remaining_generation_pins: total_pins.saturating_sub(progress.removed_generation_pins),
        failure_phase,
        failure_class: FullTextSnapshotGcFailureClass::from_error(error),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotPurgeSummary {
    removed_snapshots: usize,
    removed_staging: usize,
    removed_generation_pins: usize,
}

impl SnapshotPurgeSummary {
    pub fn removed_snapshots(self) -> usize {
        self.removed_snapshots
    }

    pub fn removed_staging(self) -> usize {
        self.removed_staging
    }

    pub fn removed_generation_pins(self) -> usize {
        self.removed_generation_pins
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTextSnapshotGcCommitReport {
    Complete(SnapshotPurgeSummary),
    Interrupted(SnapshotPurgeSummary),
    PartialFailure(SnapshotPurgePartialFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotPurgePartialFailure {
    progress: SnapshotPurgeSummary,
    remaining_snapshots: usize,
    remaining_staging: usize,
    remaining_generation_pins: usize,
    failure_phase: FullTextSnapshotGcFailurePhase,
    failure_class: FullTextSnapshotGcFailureClass,
}

impl SnapshotPurgePartialFailure {
    pub fn progress(self) -> SnapshotPurgeSummary {
        self.progress
    }

    pub fn remaining_snapshots(self) -> usize {
        self.remaining_snapshots
    }

    pub fn remaining_staging(self) -> usize {
        self.remaining_staging
    }

    pub fn remaining_generation_pins(self) -> usize {
        self.remaining_generation_pins
    }

    pub fn failure_phase(self) -> FullTextSnapshotGcFailurePhase {
        self.failure_phase
    }

    pub fn failure_class(self) -> FullTextSnapshotGcFailureClass {
        self.failure_class
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTextSnapshotGcFailurePhase {
    Preflight,
    SnapshotRemoval,
    StagingRemoval,
    SnapshotDurability,
    StagingDurability,
    PinRemoval,
    PinDurability,
    FinalValidation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTextSnapshotGcFailureClass {
    LayoutChanged,
    StorageUnavailable,
}

impl FullTextSnapshotGcFailureClass {
    fn from_error(error: &FullTextError) -> Self {
        match error {
            FullTextError::Internal { .. } => Self::LayoutChanged,
            FullTextError::Cancelled
            | FullTextError::PublicationBusy
            | FullTextError::Io { .. }
            | FullTextError::Tantivy { .. } => Self::StorageUnavailable,
        }
    }
}

struct SnapshotCandidates {
    published: BTreeSet<String>,
    snapshots: Vec<(String, PinnedSnapshotDirectory)>,
    encrypted_staging: Vec<PinnedSnapshotDirectory>,
}

fn acquire_optional_snapshot_directory(path: &Path) -> Result<Option<PinnedSnapshotDirectory>> {
    match fs::symlink_metadata(path) {
        Ok(_) => PinnedSnapshotDirectory::acquire(path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn collect_exact_staging_candidates(
    root: &Path,
    generation: &str,
    separator: &str,
) -> Result<Vec<PinnedSnapshotDirectory>> {
    validate_snapshot_directory(root)?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(root).map_err(FullTextError::io)? {
        let entry = entry.map_err(FullTextError::io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FullTextError::internal("full-text staging entry name invalid"))?;
        if !name.starts_with('.') {
            continue;
        }
        if controlled_staging_generation(&name, separator)? == generation {
            candidates.push(PinnedSnapshotDirectory::acquire(&entry.path())?);
        }
    }
    candidates.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(candidates)
}

fn collect_snapshot_candidates(
    snapshots_root: &Path,
    retained_snapshots: &BTreeSet<String>,
) -> Result<SnapshotCandidates> {
    validate_snapshot_directory(snapshots_root)?;
    let mut published = BTreeSet::new();
    let mut snapshots = Vec::new();
    let mut staging = Vec::new();
    for entry in fs::read_dir(snapshots_root).map_err(FullTextError::io)? {
        let entry = entry.map_err(FullTextError::io)?;
        let path = entry.path();
        let pinned = PinnedSnapshotDirectory::acquire(&path)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FullTextError::internal("full-text snapshot entry name invalid"))?;
        if name.starts_with('.') {
            validate_controlled_staging_name(&name, ".tmp-")?;
            staging.push(pinned);
        } else {
            validate_snapshot_name(&name)?;
            published.insert(name.clone());
            if !retained_snapshots.contains(&name) {
                snapshots.push((name, pinned));
            }
        }
    }
    snapshots.sort_by(|left, right| left.0.cmp(&right.0));
    staging.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(SnapshotCandidates {
        published,
        snapshots,
        encrypted_staging: staging,
    })
}

fn collect_staging_candidates(staging_root: &Path) -> Result<Vec<PinnedSnapshotDirectory>> {
    validate_snapshot_directory(staging_root)?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(staging_root).map_err(FullTextError::io)? {
        let entry = entry.map_err(FullTextError::io)?;
        let path = entry.path();
        let pinned = PinnedSnapshotDirectory::acquire(&path)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| FullTextError::internal("full-text staging entry name invalid"))?;
        validate_controlled_staging_name(&name, ".staging-")?;
        candidates.push(pinned);
    }
    candidates.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(candidates)
}

fn validate_controlled_staging_name(name: &str, separator: &str) -> Result<()> {
    controlled_staging_generation(name, separator).map(|_| ())
}

fn controlled_staging_generation<'name>(name: &'name str, separator: &str) -> Result<&'name str> {
    let name = name
        .strip_prefix('.')
        .ok_or_else(|| FullTextError::internal("full-text staging entry name invalid"))?;
    let (snapshot_name, suffix) = name
        .rsplit_once(separator)
        .ok_or_else(|| FullTextError::internal("full-text staging entry name invalid"))?;
    validate_snapshot_name(snapshot_name)?;
    if suffix.len() == STAGING_SUFFIX_HEX_LEN
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(snapshot_name)
    } else {
        Err(FullTextError::internal(
            "full-text staging entry name invalid",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn exact_retirement_defers_for_a_reader_and_never_collects_unrelated_artifacts() {
        let index_root = temp_dir("exact-generation-retirement");
        for generation in [
            "generation-target",
            "generation-retained",
            "generation-unrelated",
        ] {
            publish_snapshot(&index_root, generation, [document()]).unwrap();
        }
        for path in [
            index_root.join("snapshots/.generation-target.tmp-0123456789abcdef"),
            index_root.join("staging/.generation-target.staging-0123456789abcdef"),
            index_root.join("snapshots/.generation-unrelated.tmp-0123456789abcdef"),
            index_root.join("staging/.generation-unrelated.staging-0123456789abcdef"),
        ] {
            ensure_private_snapshot_directory(&path).unwrap();
        }
        let retained = BTreeSet::from(["generation-retained".to_string()]);
        let reader = FullTextIndex::open_snapshot(&index_root, "generation-target")
            .unwrap()
            .unwrap();

        assert_eq!(
            try_retire_unpublished_generation(&index_root, "generation-target", &retained).unwrap(),
            FullTextGenerationRetirement::Deferred
        );
        assert!(index_root.join("snapshots/generation-target").is_dir());
        drop(reader);

        let FullTextGenerationRetirement::Retired(summary) =
            try_retire_unpublished_generation(&index_root, "generation-target", &retained).unwrap()
        else {
            panic!("exact target was not retired");
        };
        assert!(summary.removed_snapshot());
        assert_eq!(summary.removed_staging(), 2);
        assert!(summary.removed_generation_pin());
        assert!(!index_root.join("snapshots/generation-target").exists());
        assert!(!index_root
            .join("generation-pins/generation-target.lock")
            .exists());
        for path in [
            index_root.join("snapshots/generation-retained"),
            index_root.join("snapshots/generation-unrelated"),
            index_root.join("snapshots/.generation-unrelated.tmp-0123456789abcdef"),
            index_root.join("staging/.generation-unrelated.staging-0123456789abcdef"),
            index_root.join("generation-pins/generation-retained.lock"),
            index_root.join("generation-pins/generation-unrelated.lock"),
        ] {
            assert!(
                path.exists(),
                "unrelated exact-generation artifact was removed"
            );
        }
        assert_eq!(
            try_retire_unpublished_generation(&index_root, "generation-target", &retained).unwrap(),
            FullTextGenerationRetirement::Absent
        );
        let _ = fs::remove_dir_all(index_root);
    }

    #[test]
    fn exact_retirement_rejects_a_metadata_retained_target() {
        let index_root = temp_dir("exact-retained-rejected");
        publish_snapshot(&index_root, "generation-retained", [document()]).unwrap();
        let retained = BTreeSet::from(["generation-retained".to_string()]);

        assert!(matches!(
            try_retire_unpublished_generation(&index_root, "generation-retained", &retained),
            Err(FullTextError::Internal { .. })
        ));
        assert!(index_root.join("snapshots/generation-retained").is_dir());
        assert!(index_root
            .join("generation-pins/generation-retained.lock")
            .is_file());
        let _ = fs::remove_dir_all(index_root);
    }

    #[test]
    fn late_generation_identity_replacement_fails_before_any_gc_deletion() {
        let index_root = temp_dir("late-generation-identity-replacement");
        for generation in [
            "generation-a-free",
            "generation-b-retained",
            "generation-z-replaced",
        ] {
            publish_snapshot(&index_root, generation, [document()]).unwrap();
        }
        let retained = BTreeSet::from(["generation-b-retained".to_string()]);
        let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
        let FullTextSnapshotGcPreparation::Prepared(prepared) =
            prepare_snapshot_gc(acquisition, &retained).unwrap()
        else {
            panic!("GC unexpectedly deferred");
        };
        let replaced = index_root.join("snapshots/generation-z-replaced");
        let displaced = index_root.join("generation-z-original");

        fs::rename(&replaced, &displaced).unwrap();
        ensure_private_snapshot_directory(&replaced).unwrap();
        write_private_file(&replaced.join("replacement-marker"), b"replacement").unwrap();
        let FullTextSnapshotGcCommitReport::PartialFailure(failure) = commit_snapshot_gc(prepared)
        else {
            panic!("identity replacement unexpectedly committed");
        };

        assert_eq!(
            failure.failure_class(),
            FullTextSnapshotGcFailureClass::LayoutChanged
        );
        assert_eq!(
            failure.failure_phase(),
            FullTextSnapshotGcFailurePhase::Preflight
        );
        assert_eq!(failure.progress().removed_snapshots(), 0);
        assert!(index_root.join("snapshots/generation-a-free").is_dir());
        assert!(index_root
            .join("generation-pins/generation-a-free.lock")
            .is_file());
        assert_eq!(
            fs::read(replaced.join("replacement-marker")).unwrap(),
            b"replacement"
        );
        assert!(displaced.is_dir());
        let _ = fs::remove_dir_all(index_root);
    }

    #[test]
    fn late_staging_identity_replacement_fails_before_any_gc_deletion() {
        let index_root = temp_dir("late-staging-identity-replacement");
        for generation in ["generation-a-free", "generation-b-retained"] {
            publish_snapshot(&index_root, generation, [document()]).unwrap();
        }
        let replaced = index_root.join("staging/.orphan.staging-0123456789abcdef");
        ensure_private_snapshot_directory(&replaced).unwrap();
        let retained = BTreeSet::from(["generation-b-retained".to_string()]);
        let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
        let FullTextSnapshotGcPreparation::Prepared(prepared) =
            prepare_snapshot_gc(acquisition, &retained).unwrap()
        else {
            panic!("GC unexpectedly deferred");
        };
        let displaced = index_root.join("staging-original");

        fs::rename(&replaced, &displaced).unwrap();
        ensure_private_snapshot_directory(&replaced).unwrap();
        write_private_file(&replaced.join("replacement-marker"), b"replacement").unwrap();
        let FullTextSnapshotGcCommitReport::PartialFailure(failure) = commit_snapshot_gc(prepared)
        else {
            panic!("identity replacement unexpectedly committed");
        };

        assert_eq!(
            failure.failure_class(),
            FullTextSnapshotGcFailureClass::LayoutChanged
        );
        assert_eq!(
            failure.failure_phase(),
            FullTextSnapshotGcFailurePhase::Preflight
        );
        assert_eq!(failure.progress().removed_snapshots(), 0);
        assert!(index_root.join("snapshots/generation-a-free").is_dir());
        assert_eq!(
            fs::read(replaced.join("replacement-marker")).unwrap(),
            b"replacement"
        );
        assert!(displaced.is_dir());
        let _ = fs::remove_dir_all(index_root);
    }

    #[test]
    fn partial_commit_is_reported_and_the_next_attempt_converges() {
        let index_root = temp_dir("partial-commit-converges");
        for generation in [
            "generation-a-free",
            "generation-b-retained",
            "generation-z-free",
        ] {
            publish_snapshot(&index_root, generation, [document()]).unwrap();
        }
        let retained = BTreeSet::from(["generation-b-retained".to_string()]);
        let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
        let FullTextSnapshotGcPreparation::Prepared(prepared) =
            prepare_snapshot_gc(acquisition, &retained).unwrap()
        else {
            panic!("GC unexpectedly deferred");
        };

        let FullTextSnapshotGcCommitReport::PartialFailure(failure) =
            commit_snapshot_gc_with_observer(prepared, |removed| {
                if removed == 1 {
                    Err(FullTextError::io(std::io::Error::other(
                        "synthetic commit fault",
                    )))
                } else {
                    Ok(())
                }
            })
        else {
            panic!("fault injection unexpectedly completed");
        };
        assert_eq!(failure.progress().removed_snapshots(), 1);
        assert_eq!(failure.remaining_snapshots(), 1);
        assert_eq!(
            failure.failure_class(),
            FullTextSnapshotGcFailureClass::StorageUnavailable
        );
        assert_eq!(
            failure.failure_phase(),
            FullTextSnapshotGcFailurePhase::SnapshotRemoval
        );

        let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
        let FullTextSnapshotGcPreparation::Prepared(prepared) =
            prepare_snapshot_gc(acquisition, &retained).unwrap()
        else {
            panic!("retry unexpectedly deferred");
        };
        let FullTextSnapshotGcCommitReport::Complete(summary) = commit_snapshot_gc(prepared) else {
            panic!("retry did not converge");
        };
        assert_eq!(summary.removed_snapshots(), 1);
        assert!(!index_root.join("snapshots/generation-a-free").exists());
        assert!(!index_root.join("snapshots/generation-z-free").exists());
        assert!(!index_root
            .join("generation-pins/generation-a-free.lock")
            .exists());
        assert!(!index_root
            .join("generation-pins/generation-z-free.lock")
            .exists());
        let _ = fs::remove_dir_all(index_root);
    }

    #[test]
    fn controlled_commit_interrupts_between_generation_removals() {
        let index_root = temp_dir("controlled-commit-interrupts");
        for generation in [
            "generation-a-free",
            "generation-b-retained",
            "generation-z-free",
        ] {
            publish_snapshot(&index_root, generation, [document()]).unwrap();
        }
        let retained = BTreeSet::from(["generation-b-retained".to_string()]);
        let acquisition = try_acquire_snapshot_gc(&index_root).unwrap().unwrap();
        let FullTextSnapshotGcPreparation::Prepared(prepared) =
            prepare_snapshot_gc(acquisition, &retained).unwrap()
        else {
            panic!("GC unexpectedly deferred");
        };
        let checks = std::cell::Cell::new(0_usize);
        let cancel_check = || {
            let next = checks.get() + 1;
            checks.set(next);
            next >= 3
        };

        let FullTextSnapshotGcCommitReport::Interrupted(progress) =
            commit_snapshot_gc_with_cancel_check(prepared, &cancel_check)
        else {
            panic!("controlled GC did not interrupt");
        };
        assert_eq!(progress.removed_snapshots(), 1);
        assert_eq!(checks.get(), 3);
        assert_eq!(
            ["generation-a-free", "generation-z-free"]
                .into_iter()
                .filter(|generation| index_root.join("snapshots").join(generation).exists())
                .count(),
            1
        );
        let _ = fs::remove_dir_all(index_root);
    }

    fn document() -> IndexDocument {
        IndexDocument {
            doc_id: stable_id("doc_", "gc-identity"),
            resume_version_id: stable_id("ver_", "gc-identity"),
            file_name: "synthetic.pdf".to_string(),
            clean_text: "Synthetic searchable text".to_string(),
            sections: vec![IndexSection {
                section_type: "summary".to_string(),
                text: "Synthetic searchable text".to_string(),
            }],
        }
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

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("resume-ir-fulltext-{label}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
