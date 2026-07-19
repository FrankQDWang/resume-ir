use serde::{de::IgnoredAny, Deserialize, Deserializer, Serialize};

use super::enums::{
    EvidenceLane, EvidenceStatus, PrivacyBoundary, ScanErrorClass, ScanErrorOperation,
};
use super::{decode, ensure, ensure_schema, DesktopError, SafeCount};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessState {
    Ready,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceState {
    Ready,
    Repairing,
    Unavailable,
    Degraded,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StatusState {
    Ok,
    Repairing,
    Degraded,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum IndexHealth {
    Empty,
    Building,
    Ready,
    Stale,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchRepairReason {
    MigrationRebuild,
    ArtifactUnavailable,
    SourceUnavailable,
    RuntimeInvariant,
}

#[derive(Deserialize, Serialize)]
struct Services {
    metadata: ServiceState,
    query: ServiceState,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ServiceErrorCode {
    Repairing,
    MetadataUnavailable,
    QueryServiceUnavailable,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceErrorAction {
    WaitForRepair,
    Retry,
}

#[derive(Deserialize, Serialize)]
struct ServiceError {
    code: ServiceErrorCode,
    action: ServiceErrorAction,
}

#[derive(Deserialize, Serialize)]
struct IpcMetrics {
    accepted: SafeCount,
    completed: SafeCount,
    client_disconnect: SafeCount,
    request_failure: SafeCount,
    response_failure: SafeCount,
}

#[derive(Deserialize, Serialize)]
pub(super) struct StatusBody {
    schema_version: String,
    status: StatusState,
    process_state: ProcessState,
    service_state: ServiceState,
    services: Services,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    repair_reason: Option<SearchRepairReason>,
    error: Option<ServiceError>,
    indexed_documents: Option<SafeCount>,
    searchable_documents: Option<SafeCount>,
    partial_documents: Option<SafeCount>,
    visible_epoch: Option<SafeCount>,
    failed_retryable: Option<SafeCount>,
    failed_permanent: Option<SafeCount>,
    recovery_queue_depth: Option<SafeCount>,
    ocr_queue_depth: Option<SafeCount>,
    embedding_queue_depth: Option<SafeCount>,
    entity_mentions: Option<SafeCount>,
    import_tasks_queued: Option<SafeCount>,
    index_health: Option<IndexHealth>,
    latest_import_scan: Option<LatestImportScan>,
    ipc: IpcMetrics,
}

#[derive(Deserialize, Serialize)]
struct LatestImportScan {
    files_discovered: SafeCount,
    searchable_documents: SafeCount,
    ocr_required_documents: SafeCount,
    failed_documents: SafeCount,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct DiagnosticsBody {
    schema_version: String,
    privacy_boundary: PrivacyBoundary,
    evidence_lane: EvidenceLane,
    evidence_status: EvidenceStatus,
    contains_raw_resume_text: bool,
    contains_queries: bool,
    contains_resume_paths: bool,
    contains_candidate_results: bool,
    contains_snippet_text: bool,
    visible_epoch: Option<SafeCount>,
    process_state: ProcessState,
    service_state: ServiceState,
    services: Services,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    repair_reason: Option<SearchRepairReason>,
    error: Option<ServiceError>,
    metrics: DiagnosticsMetrics,
    error_counts: DiagnosticsErrorCounts,
    #[serde(skip_serializing)]
    benchmark_refs: Vec<IgnoredAny>,
}

#[derive(Deserialize, Serialize)]
struct DiagnosticsMetrics {
    ipc: IpcMetrics,
    indexed_documents: Option<SafeCount>,
    searchable_documents: Option<SafeCount>,
    partial_documents: Option<SafeCount>,
    ocr_queue_depth: Option<SafeCount>,
    embedding_queue_depth: Option<SafeCount>,
    recovery_queue_depth: Option<SafeCount>,
    import_tasks_queued: Option<SafeCount>,
    import_tasks_recoverable: Option<SafeCount>,
    import_tasks_cancelled: Option<SafeCount>,
    query_latency: Option<QueryLatency>,
}

#[derive(Deserialize, Serialize)]
struct QueryLatency {
    sample_count: SafeCount,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
    last_result_count: Option<SafeCount>,
}

#[derive(Deserialize, Serialize)]
struct DiagnosticsErrorCounts {
    failed_retryable: Option<SafeCount>,
    failed_permanent: Option<SafeCount>,
    import_scan_errors: Option<SafeCount>,
    ocr_page_budget_blocked: Option<SafeCount>,
    ocr_language_unavailable: Option<SafeCount>,
    scan_error_buckets: Vec<ScanErrorBucket>,
}

#[derive(Deserialize, Serialize)]
struct ScanErrorBucket {
    class: ScanErrorClass,
    operation: ScanErrorOperation,
    count: SafeCount,
}

pub(super) fn project_status(body: &[u8]) -> Result<StatusBody, DesktopError> {
    let value: StatusBody = decode(body)?;
    ensure_schema(&value.schema_version, "daemon.status.v2")?;
    validate_service_contract(value.service_state, &value.services, value.error.as_ref())?;
    validate_repair_reason(&value.services, value.repair_reason)?;
    ensure(
        value.status
            == match value.service_state {
                ServiceState::Ready => StatusState::Ok,
                ServiceState::Repairing => StatusState::Repairing,
                ServiceState::Degraded | ServiceState::Unavailable => StatusState::Degraded,
            },
    )?;
    validate_metadata_counts(
        value.services.metadata,
        value.visible_epoch.is_some(),
        [
            value.indexed_documents.is_some(),
            value.searchable_documents.is_some(),
            value.partial_documents.is_some(),
            value.failed_retryable.is_some(),
            value.failed_permanent.is_some(),
            value.recovery_queue_depth.is_some(),
            value.ocr_queue_depth.is_some(),
            value.embedding_queue_depth.is_some(),
            value.entity_mentions.is_some(),
            value.import_tasks_queued.is_some(),
            value.index_health.is_some(),
        ],
    )?;
    ensure(value.services.metadata == ServiceState::Ready || value.latest_import_scan.is_none())?;
    Ok(value)
}

pub(super) fn project_diagnostics(body: &[u8]) -> Result<DiagnosticsBody, DesktopError> {
    let value: DiagnosticsBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.diagnostics.v3")?;
    validate_service_contract(value.service_state, &value.services, value.error.as_ref())?;
    validate_repair_reason(&value.services, value.repair_reason)?;
    validate_metadata_counts(
        value.services.metadata,
        value.visible_epoch.is_some(),
        [
            value.metrics.indexed_documents.is_some(),
            value.metrics.searchable_documents.is_some(),
            value.metrics.partial_documents.is_some(),
        ],
    )?;
    ensure(
        !value.contains_raw_resume_text
            && !value.contains_queries
            && !value.contains_resume_paths
            && !value.contains_candidate_results
            && !value.contains_snippet_text
            && value.benchmark_refs.len() <= 64
            && value.error_counts.scan_error_buckets.len() <= 16,
    )?;
    if let Some(latency) = &value.metrics.query_latency {
        for value in [latency.p50_ms, latency.p95_ms, latency.p99_ms]
            .into_iter()
            .flatten()
        {
            ensure(value.is_finite() && (0.0..=3_600_000.0).contains(&value))?;
        }
    }
    Ok(value)
}

fn validate_service_contract(
    aggregate: ServiceState,
    services: &Services,
    error: Option<&ServiceError>,
) -> Result<(), DesktopError> {
    ensure(!matches!(
        services.metadata,
        ServiceState::Repairing | ServiceState::Degraded
    ))?;
    ensure(services.query != ServiceState::Degraded)?;
    let expected = if services.metadata == ServiceState::Unavailable
        || services.query == ServiceState::Unavailable
    {
        ServiceState::Degraded
    } else if services.query == ServiceState::Repairing {
        ServiceState::Repairing
    } else {
        ServiceState::Ready
    };
    ensure(aggregate == expected)?;
    match (services.metadata, services.query, error) {
        (ServiceState::Ready, ServiceState::Ready, None) => Ok(()),
        (
            ServiceState::Ready,
            ServiceState::Repairing,
            Some(ServiceError {
                code: ServiceErrorCode::Repairing,
                action: ServiceErrorAction::WaitForRepair,
            }),
        ) => Ok(()),
        (
            ServiceState::Unavailable,
            ServiceState::Unavailable,
            Some(ServiceError {
                code: ServiceErrorCode::MetadataUnavailable,
                action: ServiceErrorAction::Retry,
            }),
        ) => Ok(()),
        (
            ServiceState::Ready,
            ServiceState::Unavailable,
            Some(ServiceError {
                code: ServiceErrorCode::QueryServiceUnavailable,
                action: ServiceErrorAction::Retry,
            }),
        ) => Ok(()),
        _ => ensure(false),
    }
}

fn validate_repair_reason(
    services: &Services,
    repair_reason: Option<SearchRepairReason>,
) -> Result<(), DesktopError> {
    ensure(matches!(
        (services.metadata, services.query, repair_reason),
        (ServiceState::Ready, ServiceState::Ready, None)
            | (
                ServiceState::Ready,
                ServiceState::Repairing,
                Some(
                    SearchRepairReason::MigrationRebuild | SearchRepairReason::ArtifactUnavailable
                )
            )
            | (
                ServiceState::Ready,
                ServiceState::Unavailable,
                Some(SearchRepairReason::SourceUnavailable | SearchRepairReason::RuntimeInvariant)
            )
            | (ServiceState::Unavailable, ServiceState::Unavailable, None)
    ))
}

fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn validate_metadata_counts<const N: usize>(
    metadata: ServiceState,
    has_epoch: bool,
    has_counts: [bool; N],
) -> Result<(), DesktopError> {
    match metadata {
        ServiceState::Ready => ensure(has_epoch && has_counts.into_iter().all(|present| present)),
        ServiceState::Unavailable => {
            ensure(!has_epoch && has_counts.into_iter().all(|present| !present))
        }
        ServiceState::Repairing => ensure(false),
        ServiceState::Degraded => ensure(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics_payload() -> serde_json::Value {
        serde_json::json!({
            "schema_version": "resume-ir.diagnostics.v3",
            "privacy_boundary": "redacted_local_aggregate",
            "evidence_lane": "gui_manual",
            "evidence_status": "unaccepted",
            "contains_raw_resume_text": false,
            "contains_queries": false,
            "contains_resume_paths": false,
            "contains_candidate_results": false,
            "contains_snippet_text": false,
            "visible_epoch": 4,
            "process_state": "ready",
            "service_state": "ready",
            "services": {"metadata": "ready", "query": "ready"},
            "repair_reason": null,
            "error": null,
            "metrics": {
                "ipc": {"accepted": 4, "completed": 3, "client_disconnect": 1, "request_failure": 0, "response_failure": 0},
                "indexed_documents": 4,
                "searchable_documents": 3,
                "partial_documents": 1,
                "ocr_queue_depth": 0,
                "embedding_queue_depth": 0,
                "recovery_queue_depth": 0,
                "import_tasks_queued": 0,
                "import_tasks_recoverable": 0,
                "import_tasks_cancelled": 0,
                "query_latency": {"sample_count": 1, "p50_ms": 2.0, "p95_ms": 3.0, "p99_ms": 4.0, "last_result_count": 1}
            },
            "error_counts": {"failed_retryable": 0, "failed_permanent": 0, "import_scan_errors": 0, "ocr_page_budget_blocked": 0, "ocr_language_unavailable": 0, "scan_error_buckets": []},
            "benchmark_refs": [{"private_ref": "synthetic-private-reference"}],
            "private_debug": "synthetic-private-value"
        })
    }

    #[test]
    fn status_v2_projects_ready_and_metadata_unavailable_process_health() {
        let ready = serde_json::json!({
            "schema_version": "daemon.status.v2",
            "status": "ok",
            "process_state": "ready",
            "service_state": "ready",
            "services": {"metadata": "ready", "query": "ready"},
            "repair_reason": null,
            "error": null,
            "indexed_documents": 4,
            "searchable_documents": 3,
            "partial_documents": 1,
            "visible_epoch": 7,
            "failed_retryable": 0,
            "failed_permanent": 0,
            "recovery_queue_depth": 0,
            "ocr_queue_depth": 0,
            "embedding_queue_depth": 0,
            "entity_mentions": 8,
            "import_tasks_queued": 0,
            "index_health": "ready",
            "latest_import_scan": null,
            "ipc": {"accepted": 2, "completed": 2, "client_disconnect": 0, "request_failure": 0, "response_failure": 0}
        });
        assert!(project_status(&serde_json::to_vec(&ready).unwrap()).is_ok());

        let mut missing_reason = ready.clone();
        missing_reason
            .as_object_mut()
            .unwrap()
            .remove("repair_reason");
        assert!(project_status(&serde_json::to_vec(&missing_reason).unwrap()).is_err());

        let mut ready_with_reason = ready.clone();
        ready_with_reason["repair_reason"] = serde_json::json!("migration_rebuild");
        assert!(project_status(&serde_json::to_vec(&ready_with_reason).unwrap()).is_err());

        let mut repairing = ready.clone();
        repairing["status"] = serde_json::json!("repairing");
        repairing["service_state"] = serde_json::json!("repairing");
        repairing["services"]["query"] = serde_json::json!("repairing");
        repairing["repair_reason"] = serde_json::json!("migration_rebuild");
        repairing["error"] = serde_json::json!({"code": "REPAIRING", "action": "wait_for_repair"});
        assert!(project_status(&serde_json::to_vec(&repairing).unwrap()).is_ok());
        repairing["repair_reason"] = serde_json::json!("artifact_unavailable");
        assert!(project_status(&serde_json::to_vec(&repairing).unwrap()).is_ok());
        repairing["repair_reason"] = serde_json::json!("source_unavailable");
        assert!(project_status(&serde_json::to_vec(&repairing).unwrap()).is_err());
        repairing["repair_reason"] = serde_json::Value::Null;
        assert!(project_status(&serde_json::to_vec(&repairing).unwrap()).is_err());
        repairing.as_object_mut().unwrap().remove("repair_reason");
        assert!(project_status(&serde_json::to_vec(&repairing).unwrap()).is_err());

        let mut blocked = ready.clone();
        blocked["status"] = serde_json::json!("degraded");
        blocked["service_state"] = serde_json::json!("degraded");
        blocked["services"]["query"] = serde_json::json!("unavailable");
        blocked["repair_reason"] = serde_json::json!("runtime_invariant");
        blocked["error"] =
            serde_json::json!({"code": "QUERY_SERVICE_UNAVAILABLE", "action": "retry"});
        assert!(project_status(&serde_json::to_vec(&blocked).unwrap()).is_ok());
        blocked["repair_reason"] = serde_json::json!("source_unavailable");
        assert!(project_status(&serde_json::to_vec(&blocked).unwrap()).is_ok());
        blocked["repair_reason"] = serde_json::json!("artifact_unavailable");
        assert!(project_status(&serde_json::to_vec(&blocked).unwrap()).is_err());
        blocked["repair_reason"] = serde_json::Value::Null;
        assert!(project_status(&serde_json::to_vec(&blocked).unwrap()).is_err());

        let unavailable = serde_json::json!({
            "schema_version": "daemon.status.v2",
            "status": "degraded",
            "process_state": "ready",
            "service_state": "degraded",
            "services": {"metadata": "unavailable", "query": "unavailable"},
            "repair_reason": null,
            "error": {"code": "METADATA_UNAVAILABLE", "action": "retry"},
            "indexed_documents": null,
            "searchable_documents": null,
            "partial_documents": null,
            "visible_epoch": null,
            "failed_retryable": null,
            "failed_permanent": null,
            "recovery_queue_depth": null,
            "ocr_queue_depth": null,
            "embedding_queue_depth": null,
            "entity_mentions": null,
            "import_tasks_queued": null,
            "index_health": null,
            "latest_import_scan": null,
            "ipc": {"accepted": 3, "completed": 2, "client_disconnect": 1, "request_failure": 0, "response_failure": 0}
        });
        assert!(project_status(&serde_json::to_vec(&unavailable).unwrap()).is_ok());

        let mut unavailable_with_reason = unavailable.clone();
        unavailable_with_reason["repair_reason"] = serde_json::json!("runtime_invariant");
        assert!(project_status(&serde_json::to_vec(&unavailable_with_reason).unwrap()).is_err());
    }

    #[test]
    fn diagnostics_v3_projection_drops_private_extras_and_rejects_legacy_or_unsafe_payloads() {
        let mut payload = diagnostics_payload();
        let projected = project_diagnostics(&serde_json::to_vec(&payload).unwrap()).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(!exposed.contains("benchmark_refs"));
        assert!(!exposed.contains("synthetic-private-reference"));
        assert!(!exposed.contains("private_debug"));

        let mut missing_reason = payload.clone();
        missing_reason
            .as_object_mut()
            .unwrap()
            .remove("repair_reason");
        assert!(project_diagnostics(&serde_json::to_vec(&missing_reason).unwrap()).is_err());

        let mut repairing = payload.clone();
        repairing["service_state"] = serde_json::json!("repairing");
        repairing["services"]["query"] = serde_json::json!("repairing");
        repairing["repair_reason"] = serde_json::json!("migration_rebuild");
        repairing["error"] =
            serde_json::json!({"code": "REPAIRING", "action": "wait_for_repair"});
        assert!(project_diagnostics(&serde_json::to_vec(&repairing).unwrap()).is_ok());

        let mut blocked = payload.clone();
        blocked["service_state"] = serde_json::json!("degraded");
        blocked["services"]["query"] = serde_json::json!("unavailable");
        blocked["repair_reason"] = serde_json::json!("runtime_invariant");
        blocked["error"] =
            serde_json::json!({"code": "QUERY_SERVICE_UNAVAILABLE", "action": "retry"});
        assert!(project_diagnostics(&serde_json::to_vec(&blocked).unwrap()).is_ok());

        payload["contains_queries"] = serde_json::Value::Bool(true);
        assert!(project_diagnostics(&serde_json::to_vec(&payload).unwrap()).is_err());
        payload["contains_queries"] = serde_json::Value::Bool(false);
        payload["schema_version"] = serde_json::Value::String("resume-ir.diagnostics.v2".into());
        assert!(project_diagnostics(&serde_json::to_vec(&payload).unwrap()).is_err());
    }

    #[test]
    fn unavailable_metadata_keeps_process_and_ipc_diagnostics_exportable() {
        let payload = serde_json::json!({
            "schema_version": "resume-ir.diagnostics.v3",
            "privacy_boundary": "redacted_local_aggregate",
            "evidence_lane": "gui_manual",
            "evidence_status": "unaccepted",
            "contains_raw_resume_text": false,
            "contains_queries": false,
            "contains_resume_paths": false,
            "contains_candidate_results": false,
            "contains_snippet_text": false,
            "visible_epoch": null,
            "process_state": "ready",
            "service_state": "degraded",
            "services": {"metadata": "unavailable", "query": "unavailable"},
            "repair_reason": null,
            "error": {"code": "METADATA_UNAVAILABLE", "action": "retry"},
            "metrics": {"ipc": {"accepted": 1, "completed": 1, "client_disconnect": 0, "request_failure": 0, "response_failure": 0}, "indexed_documents": null, "searchable_documents": null, "partial_documents": null},
            "error_counts": {"scan_error_buckets": []},
            "benchmark_refs": []
        });
        assert!(project_diagnostics(&serde_json::to_vec(&payload).unwrap()).is_ok());
    }
}
