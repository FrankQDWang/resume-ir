//! Project durable writer authority into public WriterHealth.

use daemon_contract::{WriterHealth, WriterReason, WriterState, WriterTransitionPhase};
use meta_store::{
    WriterAuthorityHealthState, WriterAuthoritySnapshot, WriterTransitionPhase as StorePhase,
};

pub(crate) fn writer_health_from_snapshot(snapshot: WriterAuthoritySnapshot) -> WriterHealth {
    let transition_phase = snapshot.transition_phase.map(map_phase);
    let active_transition_id = snapshot.active_transition_id.filter(|id| !id.is_empty());
    let last_completed = snapshot
        .last_completed_transition_id
        .filter(|id| !id.is_empty());
    match snapshot.health_state {
        WriterAuthorityHealthState::Ready => WriterHealth {
            state: WriterState::Ready,
            reason: None,
            // Keep WriterReady phase only when a completed transition id exists so
            // installed attestation can bind status opaque id to the private receipt.
            transition_phase: last_completed
                .as_ref()
                .map(|_| WriterTransitionPhase::WriterReady),
            transition_id: last_completed,
        },
        WriterAuthorityHealthState::Transitioning => WriterHealth {
            state: WriterState::Transitioning,
            reason: Some(map_reason(snapshot.health_reason.as_deref())),
            transition_phase: transition_phase.or(Some(WriterTransitionPhase::Observed)),
            transition_id: active_transition_id,
        },
        WriterAuthorityHealthState::Unavailable => WriterHealth {
            state: WriterState::Unavailable,
            reason: Some(map_reason(snapshot.health_reason.as_deref())),
            transition_phase: None,
            transition_id: None,
        },
        WriterAuthorityHealthState::Blocked => WriterHealth {
            state: WriterState::Blocked,
            reason: Some(map_reason(snapshot.health_reason.as_deref())),
            transition_phase: None,
            transition_id: None,
        },
    }
}

pub(crate) fn writer_health_from_store(store: &meta_store::ReadMetaStore) -> WriterHealth {
    match store.writer_authority_snapshot() {
        Ok(snapshot) => writer_health_from_snapshot(snapshot),
        Err(_) => WriterHealth::unavailable(WriterReason::PersistedStateInvalid),
    }
}

fn map_phase(phase: StorePhase) -> WriterTransitionPhase {
    match phase {
        StorePhase::Observed => WriterTransitionPhase::Observed,
        StorePhase::ClaimsFenced => WriterTransitionPhase::ClaimsFenced,
        StorePhase::WorkersQuiesced => WriterTransitionPhase::WorkersQuiesced,
        StorePhase::TargetCommitted => WriterTransitionPhase::TargetCommitted,
        StorePhase::WriterReady => WriterTransitionPhase::WriterReady,
    }
}

fn map_reason(reason: Option<&str>) -> WriterReason {
    match reason {
        Some("transition_in_progress") => WriterReason::TransitionInProgress,
        Some("runtime_unavailable") => WriterReason::RuntimeUnavailable,
        Some("unsupported_transition") => WriterReason::UnsupportedTransition,
        Some("persisted_state_invalid") => WriterReason::PersistedStateInvalid,
        Some("blocked_by_running_owner") => WriterReason::BlockedByRunningOwner,
        _ => WriterReason::RuntimeUnavailable,
    }
}
