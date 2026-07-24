use serde::{Deserialize, Serialize};

/// Closed daemon lifecycle state projected by status and diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Initializing,
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

    pub const fn blocked(reason: CoreReason) -> Self {
        Self {
            state: CoreState::Blocked,
            reason: Some(reason),
        }
    }

    pub const fn status(self) -> StatusState {
        match self.state {
            CoreState::Initializing => StatusState::Initializing,
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
}

impl OptionalRuntimeMatrix {
    pub const fn initializing() -> Self {
        Self {
            embedding: OptionalRuntimeHealth::initializing(),
            ocr: OptionalRuntimeHealth::initializing(),
            classifier: OptionalRuntimeHealth::initializing(),
        }
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
}

impl CapabilityReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CoreInitializing => "core_initializing",
            Self::CoreBlocked => "core_blocked",
            Self::EmbeddingUnavailable => "embedding_unavailable",
            Self::OcrUnavailable => "ocr_unavailable",
            Self::ClassifierUnavailable => "classifier_unavailable",
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
    pub ocr_import: CapabilityHealth,
    pub index_publication: CapabilityHealth,
}

impl CapabilityMatrix {
    pub fn derive(core: CoreHealth, runtimes: OptionalRuntimeMatrix) -> Self {
        match core.state {
            CoreState::Initializing | CoreState::Repairing => {
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
            text_import: if !classifier {
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            } else if !embedding {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            } else {
                CapabilityHealth::available()
            },
            ocr_import: if !classifier {
                CapabilityHealth::unavailable(CapabilityReason::ClassifierUnavailable)
            } else if !embedding {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            } else if !ocr {
                CapabilityHealth::unavailable(CapabilityReason::OcrUnavailable)
            } else {
                CapabilityHealth::available()
            },
            index_publication: if embedding {
                CapabilityHealth::available()
            } else {
                CapabilityHealth::unavailable(CapabilityReason::EmbeddingUnavailable)
            },
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
            CoreState::Initializing | CoreState::Repairing => Some(Self {
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
    if status != core.status()
        || !valid_core
        || ![runtimes.embedding, runtimes.ocr, runtimes.classifier]
            .into_iter()
            .all(valid_runtime)
        || capabilities != CapabilityMatrix::derive(core, runtimes)
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
