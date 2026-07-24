//! Shared typed authority for daemon health and operation-capability contracts.

mod health;

pub use health::{
    validate_health_contract, CapabilityHealth, CapabilityMatrix, CapabilityName, CapabilityReason,
    CapabilityState, ContractViolation, CoreError, CoreErrorAction, CoreErrorCode, CoreHealth,
    CoreReason, CoreState, OptionalRuntimeHealth, OptionalRuntimeMatrix, OptionalRuntimeReason,
    OptionalRuntimeState, StatusState,
};
