pub(crate) use daemon_contract::{
    CapabilityHealth, CapabilityMatrix, CapabilityState, CoreHealth, CoreReason, CoreState,
    OptionalRuntimeHealth, OptionalRuntimeMatrix, OptionalRuntimeReason, OptionalRuntimeState,
    WriterHealth,
};

pub(crate) fn health_json(
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    writer: WriterHealth,
    capabilities: CapabilityMatrix,
) -> serde_json::Value {
    serde_json::json!({
        "process_state": "ready",
        "core": core,
        "optional_runtimes": runtimes,
        "writer": writer,
        "capabilities": capabilities,
    })
}

pub(crate) fn service_error_json(core: CoreHealth) -> serde_json::Value {
    serde_json::to_value(daemon_contract::CoreError::for_core(core))
        .expect("closed daemon core error serializes")
}
