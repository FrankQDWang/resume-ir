use serde::{Deserialize, Serialize};

/// Closed daemon lifecycle state projected by status and diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Initializing,
    Migrating,
    Ok,
    Repairing,
    Degraded,
    Blocked,
}

/// State of the daemon's store-backed core.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Initializing,
    Migrating,
    Ready,
    Repairing,
    Degraded,
    Blocked,
}

/// Bounded reason attached to a non-ready core.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreReason {
    MetadataInitializing,
    MetadataMigrating,
    MigrationRebuild,
    ArtifactUnavailable,
    SourceUnavailable,
    RuntimeInvariant,
    UnsupportedStoreSchema,
    MetadataUnavailable,
}

impl CoreReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MetadataInitializing => "metadata_initializing",
            Self::MetadataMigrating => "metadata_migrating",
            Self::MigrationRebuild => "migration_rebuild",
            Self::ArtifactUnavailable => "artifact_unavailable",
            Self::SourceUnavailable => "source_unavailable",
            Self::RuntimeInvariant => "runtime_invariant",
            Self::UnsupportedStoreSchema => "unsupported_store_schema",
            Self::MetadataUnavailable => "metadata_unavailable",
        }
    }
}

/// Core health shared by the producer and every native consumer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreHealth {
    pub state: CoreState,
    #[serde(deserialize_with = "required_nullable")]
    pub reason: Option<CoreReason>,
}

impl CoreHealth {
    pub const fn initializing() -> Self {
        Self {
            state: CoreState::Initializing,
            reason: Some(CoreReason::MetadataInitializing),
        }
    }

    pub const fn ready() -> Self {
        Self {
            state: CoreState::Ready,
            reason: None,
        }
    }

    pub const fn migrating() -> Self {
        Self {
            state: CoreState::Migrating,
            reason: Some(CoreReason::MetadataMigrating),
        }
    }

    pub const fn blocked(reason: CoreReason) -> Self {
        Self {
            state: CoreState::Blocked,
            reason: Some(reason),
        }
    }

    pub const fn status(self) -> StatusState {
        match self.state {
            CoreState::Initializing => StatusState::Initializing,
            CoreState::Migrating => StatusState::Migrating,
            CoreState::Ready => StatusState::Ok,
            CoreState::Repairing => StatusState::Repairing,
            CoreState::Degraded => StatusState::Degraded,
            CoreState::Blocked => StatusState::Blocked,
        }
    }
}

/// State of one optional runtime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionalRuntimeState {
    Initializing,
    Available,
    Unavailable,
}

/// Closed reason for an unavailable optional runtime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionalRuntimeReason {
    Missing,
    Invalid,
    StartFailed,
    NotConfigured,
}

impl OptionalRuntimeReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::StartFailed => "start_failed",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// Health of one optional runtime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalRuntimeHealth {
    pub state: OptionalRuntimeState,
    #[serde(deserialize_with = "required_nullable")]
    pub reason: Option<OptionalRuntimeReason>,
}

impl OptionalRuntimeHealth {
    pub const fn initializing() -> Self {
        Self {
            state: OptionalRuntimeState::Initializing,
            reason: None,
        }
    }

    pub const fn available() -> Self {
        Self {
            state: OptionalRuntimeState::Available,
            reason: None,
        }
    }

    pub const fn unavailable(reason: OptionalRuntimeReason) -> Self {
        Self {
            state: OptionalRuntimeState::Unavailable,
            reason: Some(reason),
        }
    }

    const fn is_available(self) -> bool {
        matches!(self.state, OptionalRuntimeState::Available)
    }
}

/// Fixed optional-runtime matrix.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalRuntimeMatrix {
    pub embedding: OptionalRuntimeHealth,
    pub ocr: OptionalRuntimeHealth,
    pub classifier: OptionalRuntimeHealth,
    pub pdfium: OptionalRuntimeHealth,
}

impl OptionalRuntimeMatrix {
    pub const fn initializing() -> Self {
        Self {
            embedding: OptionalRuntimeHealth::initializing(),
            ocr: OptionalRuntimeHealth::initializing(),
            classifier: OptionalRuntimeHealth::initializing(),
            pdfium: OptionalRuntimeHealth::initializing(),
        }
    }
}

/// Independent writer-authority health; never folded into CoreReason.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterState {
    Ready,
    Transitioning,
    Unavailable,
    Blocked,
}

/// Bounded reason for a non-ready writer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterReason {
    TransitionInProgress,
    RuntimeUnavailable,
    UnsupportedTransition,
    PersistedStateInvalid,
    BlockedByRunningOwner,
}

impl WriterReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TransitionInProgress => "transition_in_progress",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::UnsupportedTransition => "unsupported_transition",
            Self::PersistedStateInvalid => "persisted_state_invalid",
            Self::BlockedByRunningOwner => "blocked_by_running_owner",
        }
    }
}

/// Durable writer transition phase projected to status/diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterTransitionPhase {
    Observed,
    ClaimsFenced,
    WorkersQuiesced,
    TargetCommitted,
    WriterReady,
}

/// Writer authority health shared by status, diagnostics, and capability derive.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WriterHealth {
    pub state: WriterState,
    #[serde(deserialize_with = "required_nullable")]
    pub reason: Option<WriterReason>,
    #[serde(deserialize_with = "required_nullable")]
    pub transition_phase: Option<WriterTransitionPhase>,
}

impl WriterHealth {
    pub const fn ready() -> Self {
        Self {
            state: WriterState::Ready,
            reason: None,
            transition_phase: None,
        }
    }

    pub const fn transitioning(phase: WriterTransitionPhase) -> Self {
        Self {
            state: WriterState::Transitioning,
            reason: Some(WriterReason::TransitionInProgress),
            transition_phase: Some(phase),
        }
    }

    pub const fn unavailable(reason: WriterReason) -> Self {
        Self {
            state: WriterState::Unavailable,
            reason: Some(reason),
            transition_phase: None,
        }
    }

    pub const fn blocked(reason: WriterReason) -> Self {
        Self {
            state: WriterState::Blocked,
            reason: Some(reason),
            transition_phase: None,
        }
    }

    pub const fn admits_public_writers(self) -> bool {
        matches!(self.state, WriterState::Ready)
    }
}

/// State of one public operation capability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Initializing,
    Available,
    Degraded,
    Unavailable,
    Blocked,
}

impl CapabilityState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::Blocked => "blocked",
        }
    }
}

/// Closed dependency reason for a non-available capability.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    CoreInitializing,
    CoreBlocked,
    EmbeddingUnavailable,
    OcrUnavailable,
    ClassifierUnavailable,
    PdfiumUnavailable,
    WriterUnavailable,
    WriterTransitioning,
}

impl CapabilityReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CoreInitializing => "core_initializing",
            Self::CoreBlocked => "core_blocked",
            Self::EmbeddingUnavailable => "embedding_unavailable",
            Self::OcrUnavailable => "ocr_unavailable",
            Self::ClassifierUnavailable => "classifier_unavailable",
            Self::PdfiumUnavailable => "pdfium_unavailable",
            Self::WriterUnavailable => "writer_unavailable",
            Self::WriterTransitioning => "writer_transitioning",
        }
    }
}

/// State and dependency reason for one public operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityHealth {
    pub state: CapabilityState,
    #[serde(deserialize_with = "required_nullable")]
    pub reason: Option<CapabilityReason>,
}

impl CapabilityHealth {
    const fn available() -> Self {
        Self {
            state: CapabilityState::Available,
            reason: None,
        }
    }

    const fn degraded(reason: CapabilityReason) -> Self {
        Self {
            state: CapabilityState::Degraded,
            reason: Some(reason),
        }
    }

    const fn unavailable(reason: CapabilityReason) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            reason: Some(reason),
        }
    }
}

/// Fixed public operation-capability matrix.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMatrix {
    pub keyword_search: CapabilityHealth,
    pub detail: CapabilityHealth,
    pub semantic_search: CapabilityHealth,
    pub hybrid_search: CapabilityHealth,
    pub text_import: CapabilityHealth,
    pub pdf_import: CapabilityHealth,
    pub ocr_import: CapabilityHealth,
    pub index_publication: CapabilityHealth,
}

impl CapabilityMatrix {
    pub fn derive(core: CoreHealth, runtimes: OptionalRuntimeMatrix, writer: WriterHealth) -> Self {
        match core.state {
            CoreState::Initializing | CoreState::Migrating | CoreState::Repairing => {
                return Self::uniform(
                    CapabilityState::Initializing,
                    CapabilityReason::CoreInitializing,
                );
            }
            CoreState::Degraded | CoreState::Blocked => {
                return Self::uniform(CapabilityState::Blocked, CapabilityReason::CoreBlocked);
            }
            CoreState::Ready => {}
        }

        let embedding = runtimes.embedding.is_available();
        let classifier = runtimes.classifier.is_available();
        let ocr = runtimes.ocr.is_available();
        let pdfium = runtimes.pdfium.is_available();
        let writer_gate = match writer.state {
            WriterState::Ready => None,
            WriterState::Transitioning => Some(CapabilityReason::WriterTransitioning),
            WriterState::Unavailable | WriterState::Blocked => {
                Some(CapabilityReason::WriterUnavailable)
            }
        };
        let gated_import = |available: CapabilityHealth| -> CapabilityHealth {
            match writer_gate {
                None => available,
                Some(reason) => CapabilityHealth {
                    state: CapabilityState::Blocked,
                    reason: Some(reason),
                },
            }
        };
        Self {
            keyword_search: CapabilityHealth::available(),
            detail: CapabilityHealth::available(),
            semantic_search: if embedding {
                CapabilityHealth::available()
            } else {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            },
            hybrid_search: if embedding {
                CapabilityHealth::available()
            } else {
                CapabilityHealth::degraded(CapabilityReason::EmbeddingUnavailable)
            },
            text_import: gated_import(if !classifier {
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            } else if !embedding {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            } else {
                CapabilityHealth::available()
            }),
            pdf_import: gated_import(if !classifier {
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            } else if !embedding {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            } else if !pdfium {
                CapabilityHealth::unavailable(CapabilityReason::PdfiumUnavailable)
            } else {
                CapabilityHealth::available()
            }),
            ocr_import: gated_import(if !classifier {
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            } else if !embedding {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            } else if !pdfium {
                CapabilityHealth::unavailable(CapabilityReason::PdfiumUnavailable)
            } else if !ocr {
                CapabilityHealth::unavailable(CapabilityReason::OcrUnavailable)
            } else {
                CapabilityHealth::available()
            }),
            // Public index publication follows writer admission; search-authority
            // artifact repair and privacy deletion use WriterAuthorityToken paths.
            index_publication: gated_import(if embedding {
                CapabilityHealth::available()
            } else {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            }),
        }
    }

    fn uniform(state: CapabilityState, reason: CapabilityReason) -> Self {
        let health = CapabilityHealth {
            state,
            reason: Some(reason),
        };
        Self {
            keyword_search: health,
            detail: health,
            semantic_search: health,
            hybrid_search: health,
            text_import: health,
            pdf_import: health,
            ocr_import: health,
            index_publication: health,
        }
    }
}

/// Capability name carried by typed service errors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityName {
    KeywordSearch,
    Detail,
    SemanticSearch,
    HybridSearch,
    TextImport,
    PdfImport,
    OcrImport,
    IndexPublication,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoreErrorCode {
    ServiceInitializing,
    ServiceBlocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreErrorAction {
    WaitForService,
    Retry,
    RepairRequired,
}

/// Typed core error embedded in status and diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreError {
    pub code: CoreErrorCode,
    pub action: CoreErrorAction,
    #[serde(deserialize_with = "required_nullable")]
    pub capability: Option<CapabilityName>,
    pub reason: CoreReason,
}

impl CoreError {
    pub fn for_core(core: CoreHealth) -> Option<Self> {
        let reason = core.reason?;
        match core.state {
            CoreState::Initializing | CoreState::Migrating | CoreState::Repairing => Some(Self {
                code: CoreErrorCode::ServiceInitializing,
                action: CoreErrorAction::WaitForService,
                capability: None,
                reason,
            }),
            CoreState::Degraded => Some(Self {
                code: CoreErrorCode::ServiceBlocked,
                action: CoreErrorAction::Retry,
                capability: None,
                reason,
            }),
            CoreState::Blocked => Some(Self {
                code: CoreErrorCode::ServiceBlocked,
                action: CoreErrorAction::RepairRequired,
                capability: None,
                reason,
            }),
            CoreState::Ready => None,
        }
    }
}

/// Closed validation failure; it deliberately carries no raw payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractViolation;

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("daemon health contract is invalid")
    }
}

impl std::error::Error for ContractViolation {}

pub fn validate_health_contract(
    status: StatusState,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    writer: WriterHealth,
    capabilities: CapabilityMatrix,
    error: Option<CoreError>,
) -> Result<(), ContractViolation> {
    let valid_core = matches!(
        (core.state, core.reason),
        (CoreState::Ready, None)
            | (
                CoreState::Initializing,
                Some(CoreReason::MetadataInitializing)
            )
            | (CoreState::Migrating, Some(CoreReason::MetadataMigrating))
            | (
                CoreState::Repairing,
                Some(CoreReason::MigrationRebuild | CoreReason::ArtifactUnavailable)
            )
            | (
                CoreState::Degraded | CoreState::Blocked,
                Some(
                    CoreReason::ArtifactUnavailable
                        | CoreReason::SourceUnavailable
                        | CoreReason::RuntimeInvariant
                        | CoreReason::UnsupportedStoreSchema
                        | CoreReason::MetadataUnavailable
                )
            )
    );
    let valid_runtime = |runtime: OptionalRuntimeHealth| {
        matches!(
            (runtime.state, runtime.reason),
            (
                OptionalRuntimeState::Initializing | OptionalRuntimeState::Available,
                None
            ) | (OptionalRuntimeState::Unavailable, Some(_))
        )
    };
    let valid_writer = matches!(
        (writer.state, writer.reason),
        (WriterState::Ready, None)
            | (
                WriterState::Transitioning,
                Some(WriterReason::TransitionInProgress)
            )
            | (
                WriterState::Unavailable | WriterState::Blocked,
                Some(
                    WriterReason::RuntimeUnavailable
                        | WriterReason::UnsupportedTransition
                        | WriterReason::PersistedStateInvalid
                        | WriterReason::BlockedByRunningOwner
                        | WriterReason::TransitionInProgress
                )
            )
    );
    if status != core.status()
        || !valid_core
        || !valid_writer
        || ![
            runtimes.embedding,
            runtimes.ocr,
            runtimes.classifier,
            runtimes.pdfium,
        ]
        .into_iter()
        .all(valid_runtime)
        || capabilities != CapabilityMatrix::derive(core, runtimes, writer)
        || error != CoreError::for_core(core)
    {
        return Err(ContractViolation);
    }
    Ok(())
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
