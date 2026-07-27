use crate::{
    contract_delta::{ContractDelta, ContractTransitionStrategy},
    ImportProcessingContract, ImportProcessingContractId,
};

/// Attempt-level or terminal observation from the writer-transition API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterContractTransitionOutcome {
    /// Committed writer contract already matches Desired.
    AlreadyActive,
    /// This call completed TargetCommitted (and optional WriterReady).
    TargetCommitted,
    TransitionRequired,
    TransitionInProgress,
    BlockedByRunningOwner,
    PersistedStateInvalid,
    UnsupportedTransition,
    RuntimeUnavailable,
}

/// Durable writer-transition phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterTransitionPhase {
    Observed,
    ClaimsFenced,
    WorkersQuiesced,
    TargetCommitted,
    WriterReady,
}

impl WriterTransitionPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::ClaimsFenced => "claims_fenced",
            Self::WorkersQuiesced => "workers_quiesced",
            Self::TargetCommitted => "target_committed",
            Self::WriterReady => "writer_ready",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "observed" => Some(Self::Observed),
            "claims_fenced" => Some(Self::ClaimsFenced),
            "workers_quiesced" => Some(Self::WorkersQuiesced),
            "target_committed" => Some(Self::TargetCommitted),
            "writer_ready" => Some(Self::WriterReady),
            _ => None,
        }
    }
}

/// Public/redacted writer health projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterAuthorityHealthState {
    Ready,
    Transitioning,
    Unavailable,
    Blocked,
}

impl WriterAuthorityHealthState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Transitioning => "transitioning",
            Self::Unavailable => "unavailable",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "transitioning" => Some(Self::Transitioning),
            "unavailable" => Some(Self::Unavailable),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

/// Snapshot of writer authority used by dormant APIs and later coordinator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterAuthoritySnapshot {
    pub health_state: WriterAuthorityHealthState,
    pub health_reason: Option<String>,
    pub transition_phase: Option<WriterTransitionPhase>,
    pub active_transition_id: Option<String>,
    pub last_completed_transition_id: Option<String>,
    pub claim_fence_epoch: u64,
    pub committed_contract_id: Option<ImportProcessingContractId>,
    pub desired_contract_id: Option<ImportProcessingContractId>,
}

/// Bounded internal transition receipt. Full digests stay private to the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterTransitionReceipt {
    pub transition_id: String,
    pub phase: WriterTransitionPhase,
    pub attempt: u32,
    pub claim_fence_epoch: u64,
    pub failure_class: Option<&'static str>,
    pub retryable: bool,
    pub campaign_id: Option<String>,
}

/// Plans whether Desired requires an online transition without mutating state.
pub fn observe_writer_contract_transition(
    committed: Option<&ImportProcessingContract>,
    desired: &ImportProcessingContract,
    running_task_count: u64,
) -> (ContractDelta, WriterContractTransitionOutcome) {
    let delta = ContractDelta::between(committed, desired);
    if delta.strategy == ContractTransitionStrategy::AlreadyActive {
        return (delta, WriterContractTransitionOutcome::AlreadyActive);
    }
    if delta.strategy == ContractTransitionStrategy::Unsupported {
        return (
            delta,
            WriterContractTransitionOutcome::UnsupportedTransition,
        );
    }
    if running_task_count > 0 {
        return (
            delta,
            WriterContractTransitionOutcome::BlockedByRunningOwner,
        );
    }
    (delta, WriterContractTransitionOutcome::TransitionRequired)
}

/// Maps a campaign strategy onto the durable affected-domain label.
pub fn campaign_domain_for(strategy: ContractTransitionStrategy) -> Option<&'static str> {
    match strategy {
        ContractTransitionStrategy::AlreadyActive => None,
        ContractTransitionStrategy::PdfRootRescan => Some("pdf_root_rescan"),
        ContractTransitionStrategy::OcrRequeue => Some("ocr_requeue"),
        ContractTransitionStrategy::ClassifierReclassify => Some("classifier_reclassify"),
        ContractTransitionStrategy::DerivedRebuild => Some("derived_rebuild"),
        ContractTransitionStrategy::Unsupported => Some("unsupported"),
    }
}

#[cfg(test)]
#[path = "processing_contract_transition_tests.rs"]
mod tests;
