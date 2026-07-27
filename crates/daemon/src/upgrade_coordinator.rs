//! Upgrade coordinator: online processing-contract writer transitions.
//!
//! Claim fence precedes quiesce and target commit. Hard-cut activation remains
//! only for unpublished migration rebuild / repair_blocked paths.

use meta_store::{
    observe_writer_contract_transition, ImportProcessingContract, OwnedMetaStore, UnixTimestamp,
    WriterContractTransitionOutcome,
};

use super::{import_processing, DaemonError, Result};

/// Opaque token proving coordinator-authorized mutation (not a public status bit).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WriterAuthorityToken {
    claim_fence_epoch: u64,
}

#[allow(dead_code)]
impl WriterAuthorityToken {
    pub(crate) const fn claim_fence_epoch(self) -> u64 {
        self.claim_fence_epoch
    }
}

/// Priority ladder for background writers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[allow(dead_code)]
pub(crate) enum WriterPriority {
    MetadataRecovery = 1,
    SearchArtifactRepair = 2,
    PrivacyDeletion = 3,
    WriterContractTransition = 4,
    OrdinaryImport = 5,
}

/// Outcome of one coordinator observe/bootstrap pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpgradeCoordinatorOutcome {
    AlreadyActive,
    TransitionRequired,
    TransitionInProgress,
    WriterUnavailable,
    HardCutActivated,
}

/// Observes Desired vs committed writer contract without mutating dormant v34
/// tables during Slice B/C. Slice D callers use this to decide fencing.
pub(crate) fn observe_desired_contract(
    store: &OwnedMetaStore,
    desired: &ImportProcessingContract,
) -> Result<(UpgradeCoordinatorOutcome, Option<WriterAuthorityToken>)> {
    let committed = store
        .active_import_processing_contract()
        .map_err(DaemonError::store)?;
    let running = store
        .running_import_task_count()
        .map_err(DaemonError::store)?;
    let (_delta, outcome) =
        observe_writer_contract_transition(committed.as_ref(), desired, running);
    match outcome {
        WriterContractTransitionOutcome::AlreadyActive => Ok((
            UpgradeCoordinatorOutcome::AlreadyActive,
            Some(WriterAuthorityToken {
                claim_fence_epoch: 0,
            }),
        )),
        WriterContractTransitionOutcome::TransitionRequired => {
            Ok((UpgradeCoordinatorOutcome::TransitionRequired, None))
        }
        WriterContractTransitionOutcome::TransitionInProgress => {
            Ok((UpgradeCoordinatorOutcome::TransitionInProgress, None))
        }
        WriterContractTransitionOutcome::BlockedByRunningOwner => {
            Ok((UpgradeCoordinatorOutcome::TransitionInProgress, None))
        }
        WriterContractTransitionOutcome::UnsupportedTransition
        | WriterContractTransitionOutcome::RuntimeUnavailable
        | WriterContractTransitionOutcome::PersistedStateInvalid => {
            Ok((UpgradeCoordinatorOutcome::WriterUnavailable, None))
        }
    }
}

/// Bootstrap writer barrier used after store open.
///
/// Until Slice D fully enables online transition commits, this still activates
/// via the legacy hard-cut path for unpublished rebuilds, but never treats
/// `Superseded` on a Ready store as a committed online transition. Ready stores
/// with a matching contract receive a writer token; mismatched Ready contracts
/// report `TransitionRequired` without blocking search.
pub(crate) fn bootstrap_writer_barrier(
    store: &OwnedMetaStore,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<UpgradeCoordinatorOutcome> {
    let (outcome, _token) = observe_desired_contract(store, desired)?;
    match outcome {
        UpgradeCoordinatorOutcome::AlreadyActive => {
            Ok(UpgradeCoordinatorOutcome::AlreadyActive)
        }
        UpgradeCoordinatorOutcome::TransitionRequired => {
            // Prefer online commit for Ready published authorities. Fall back to
            // hard-cut only when the projection still matches rebuild preconditions.
            match store
                .commit_online_writer_contract(desired, now)
                .map_err(DaemonError::store)?
            {
                WriterContractTransitionOutcome::AlreadyActive => {
                    return Ok(UpgradeCoordinatorOutcome::AlreadyActive);
                }
                WriterContractTransitionOutcome::BlockedByRunningOwner => {
                    return Ok(UpgradeCoordinatorOutcome::TransitionInProgress);
                }
                WriterContractTransitionOutcome::PersistedStateInvalid => {}
                other => {
                    let _ = other;
                }
            }
            match store
                .activate_migration_rebuild_contract(desired, now)
                .map_err(DaemonError::store)?
            {
                meta_store::MigrationRebuildContractActivation::Activated
                | meta_store::MigrationRebuildContractActivation::AlreadyActive => {
                    Ok(UpgradeCoordinatorOutcome::HardCutActivated)
                }
                meta_store::MigrationRebuildContractActivation::Superseded => {
                    Ok(UpgradeCoordinatorOutcome::TransitionRequired)
                }
                meta_store::MigrationRebuildContractActivation::RunningTaskConflict => {
                    Ok(UpgradeCoordinatorOutcome::TransitionInProgress)
                }
            }
        }
        other => Ok(other),
    }
}

/// Whether public/uncoordinated writers may claim work.
pub(crate) fn public_writer_admitted(outcome: UpgradeCoordinatorOutcome) -> bool {
    matches!(
        outcome,
        UpgradeCoordinatorOutcome::AlreadyActive | UpgradeCoordinatorOutcome::HardCutActivated
    )
}

/// Convenience: derive Desired from run options.
#[allow(dead_code)]
pub(crate) fn desired_contract(
    options: &super::RunOptions,
) -> Result<ImportProcessingContract> {
    import_processing::current_contract(options)
}
