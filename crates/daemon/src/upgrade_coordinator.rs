//! Upgrade coordinator: online processing-contract writer transitions.
//!
//! Claim fence precedes quiesce and target commit. Hard-cut activation remains
//! only for unpublished migration rebuild / repair_blocked paths.

use meta_store::{
    observe_writer_contract_transition, ImportProcessingContract, OwnedMetaStore, UnixTimestamp,
    WriterAuthorityHealthState, WriterContractTransitionOutcome,
};

use super::{import_processing, DaemonError, Result};

/// Opaque token proving coordinator-authorized mutation (not a public status bit).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WriterAuthorityToken {
    claim_fence_epoch: u64,
}

impl WriterAuthorityToken {
    #[allow(dead_code)]
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
    TargetCommitted,
    TransitionRequired,
    TransitionInProgress,
    WriterUnavailable,
    HardCutActivated,
}

/// Observes Desired vs committed writer contract.
pub(crate) fn observe_desired_contract(
    store: &OwnedMetaStore,
    desired: &ImportProcessingContract,
) -> Result<(UpgradeCoordinatorOutcome, Option<WriterAuthorityToken>)> {
    let authority = store
        .writer_authority_snapshot()
        .map_err(DaemonError::store)?;
    if authority.health_state == WriterAuthorityHealthState::Unavailable
        || authority.health_state == WriterAuthorityHealthState::Blocked
    {
        return Ok((UpgradeCoordinatorOutcome::WriterUnavailable, None));
    }
    let committed = store
        .active_import_processing_contract()
        .map_err(DaemonError::store)?;
    if committed.is_none() {
        // A fresh unpublished store has no writer head yet. Bootstrap owns the
        // existing hard-cut activation path; public enqueue must not invent it.
        return Ok((UpgradeCoordinatorOutcome::TransitionRequired, None));
    }
    let running = store
        .running_import_task_count()
        .map_err(DaemonError::store)?;
    let (delta, outcome) = observe_writer_contract_transition(committed.as_ref(), desired, running);
    let _ = delta;
    let token = WriterAuthorityToken {
        claim_fence_epoch: authority.claim_fence_epoch,
    };
    match outcome {
        WriterContractTransitionOutcome::AlreadyActive => {
            Ok((UpgradeCoordinatorOutcome::AlreadyActive, Some(token)))
        }
        WriterContractTransitionOutcome::TargetCommitted => {
            Ok((UpgradeCoordinatorOutcome::TargetCommitted, Some(token)))
        }
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
pub(crate) fn bootstrap_writer_barrier(
    store: &OwnedMetaStore,
    desired: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<(UpgradeCoordinatorOutcome, Option<WriterAuthorityToken>)> {
    store
        .reconcile_writer_runtime_availability(/*runtime_healthy*/ true, now)
        .map_err(DaemonError::store)?;
    store
        .clear_writer_unsupported_transition(now)
        .map_err(DaemonError::store)?;
    let (outcome, token) = observe_desired_contract(store, desired)?;
    match outcome {
        UpgradeCoordinatorOutcome::AlreadyActive | UpgradeCoordinatorOutcome::TargetCommitted => {
            Ok((outcome, token))
        }
        UpgradeCoordinatorOutcome::TransitionRequired => {
            match store
                .complete_online_writer_transition(desired, now)
                .map_err(DaemonError::store)?
            {
                WriterContractTransitionOutcome::AlreadyActive => {
                    let token = token_from_store(store)?;
                    return Ok((UpgradeCoordinatorOutcome::AlreadyActive, Some(token)));
                }
                WriterContractTransitionOutcome::TargetCommitted => {
                    let token = token_from_store(store)?;
                    return Ok((UpgradeCoordinatorOutcome::TargetCommitted, Some(token)));
                }
                WriterContractTransitionOutcome::BlockedByRunningOwner => {
                    return Ok((UpgradeCoordinatorOutcome::TransitionInProgress, None));
                }
                WriterContractTransitionOutcome::PersistedStateInvalid => {}
                WriterContractTransitionOutcome::TransitionInProgress => {
                    return Ok((UpgradeCoordinatorOutcome::TransitionInProgress, None));
                }
                WriterContractTransitionOutcome::UnsupportedTransition => {
                    store
                        .mark_writer_unsupported_transition(now)
                        .map_err(DaemonError::store)?;
                    return Ok((UpgradeCoordinatorOutcome::WriterUnavailable, None));
                }
                WriterContractTransitionOutcome::TransitionRequired
                | WriterContractTransitionOutcome::RuntimeUnavailable => {
                    return Ok((UpgradeCoordinatorOutcome::WriterUnavailable, None));
                }
            }
            match store
                .activate_migration_rebuild_contract(desired, now)
                .map_err(DaemonError::store)?
            {
                meta_store::MigrationRebuildContractActivation::Activated
                | meta_store::MigrationRebuildContractActivation::AlreadyActive => {
                    let token = token_from_store(store)?;
                    Ok((UpgradeCoordinatorOutcome::HardCutActivated, Some(token)))
                }
                meta_store::MigrationRebuildContractActivation::Superseded => {
                    let active = store
                        .active_import_processing_contract()
                        .map_err(DaemonError::store)?;
                    if active
                        .as_ref()
                        .is_some_and(|active| active.id() == desired.id())
                    {
                        let token = token_from_store(store)?;
                        Ok((UpgradeCoordinatorOutcome::AlreadyActive, Some(token)))
                    } else {
                        Ok((UpgradeCoordinatorOutcome::TransitionRequired, None))
                    }
                }
                meta_store::MigrationRebuildContractActivation::RunningTaskConflict => {
                    Ok((UpgradeCoordinatorOutcome::TransitionInProgress, None))
                }
            }
        }
        UpgradeCoordinatorOutcome::WriterUnavailable => {
            store
                .mark_writer_unsupported_transition(now)
                .map_err(DaemonError::store)?;
            Ok((UpgradeCoordinatorOutcome::WriterUnavailable, None))
        }
        other => Ok((other, None)),
    }
}

fn token_from_store(store: &OwnedMetaStore) -> Result<WriterAuthorityToken> {
    let authority = store
        .writer_authority_snapshot()
        .map_err(DaemonError::store)?;
    Ok(WriterAuthorityToken {
        claim_fence_epoch: authority.claim_fence_epoch,
    })
}

/// Whether public/uncoordinated writers may claim work.
pub(crate) fn public_writer_admitted(outcome: UpgradeCoordinatorOutcome) -> bool {
    matches!(
        outcome,
        UpgradeCoordinatorOutcome::AlreadyActive
            | UpgradeCoordinatorOutcome::TargetCommitted
            | UpgradeCoordinatorOutcome::HardCutActivated
    )
}

pub(crate) fn admits_priority(
    outcome: UpgradeCoordinatorOutcome,
    priority: WriterPriority,
) -> bool {
    match priority {
        WriterPriority::MetadataRecovery
        | WriterPriority::SearchArtifactRepair
        | WriterPriority::PrivacyDeletion => true,
        WriterPriority::WriterContractTransition => {
            !matches!(outcome, UpgradeCoordinatorOutcome::WriterUnavailable)
        }
        WriterPriority::OrdinaryImport => public_writer_admitted(outcome),
    }
}

/// Convenience: derive Desired from run options.
#[allow(dead_code)]
pub(crate) fn desired_contract(options: &super::RunOptions) -> Result<ImportProcessingContract> {
    import_processing::current_contract(options)
}
