use serde::{Deserialize, Serialize};

use super::health_contract::{
    deserialize_required_nullable, validate_counts, validate_health_contract, validate_latency,
    validate_repair_progress, Capabilities, CoreError, CoreStatus, IpcMetrics, OptionalRuntimes,
    ProcessState, RepairProgress, StatusState,
};
use super::{decode, ensure, ensure_schema, DesktopError, SafeCount};

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
enum ActiveProfile {
    Balanced,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatusBody {
    schema_version: String,
    status: StatusState,
    process_state: ProcessState,
    core: CoreStatus,
    optional_runtimes: OptionalRuntimes,
    capabilities: Capabilities,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    error: Option<CoreError>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    repair_progress: Option<RepairProgress>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    indexed_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    searchable_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    partial_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    visible_epoch: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    failed_retryable: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    failed_permanent: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    recovery_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_jobs_queued: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_page_budget_blocked: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_remediation: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_language_unavailable: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_language_remediation: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    embedding_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    entity_mentions: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_queued: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_recoverable: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_cancelled: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_scan_scopes: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_scan_errors: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    query_latency: Option<StatusQueryLatency>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    index_health: Option<IndexHealth>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    latest_import_scan: Option<LatestImportScan>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    active_profile: Option<ActiveProfile>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    snapshot_present: Option<bool>,
    ipc: IpcMetrics,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LatestImportScan {
    scan_profile: ScanProfile,
    files_discovered: SafeCount,
    ignored_entries: SafeCount,
    scan_errors: SafeCount,
    searchable_documents: SafeCount,
    ocr_required_documents: SafeCount,
    ocr_jobs_queued: SafeCount,
    failed_documents: SafeCount,
    deleted_documents: SafeCount,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    scan_budget_observed: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    scan_budget_limit: Option<SafeCount>,
    scan_budget_exhausted: bool,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ScanProfile {
    Explicit,
    Discovery,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StatusQueryLatency {
    sample_count: SafeCount,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p50_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p95_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p99_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    last_result_count: Option<SafeCount>,
    raw_queries: Redacted,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
enum Redacted {
    #[serde(rename = "<redacted>")]
    Value,
}

pub(super) fn project_status(body: &[u8]) -> Result<StatusBody, DesktopError> {
    let value: StatusBody = decode(body)?;
    ensure_schema(&value.schema_version, "daemon.status.v3")?;
    validate_health_contract(
        value.status,
        &value.core,
        &value.optional_runtimes,
        &value.capabilities,
        value.error.as_ref(),
    )?;
    validate_counts(
        value.visible_epoch.is_some(),
        [
            value.indexed_documents.is_some(),
            value.searchable_documents.is_some(),
            value.partial_documents.is_some(),
            value.failed_retryable.is_some(),
            value.failed_permanent.is_some(),
            value.recovery_queue_depth.is_some(),
            value.ocr_queue_depth.is_some(),
            value.ocr_jobs_queued.is_some(),
            value.ocr_page_budget_blocked.is_some(),
            value.ocr_remediation.is_some(),
            value.ocr_language_unavailable.is_some(),
            value.ocr_language_remediation.is_some(),
            value.embedding_queue_depth.is_some(),
            value.entity_mentions.is_some(),
            value.import_tasks_queued.is_some(),
            value.import_tasks_recoverable.is_some(),
            value.import_tasks_cancelled.is_some(),
            value.import_scan_scopes.is_some(),
            value.import_scan_errors.is_some(),
            value.query_latency.is_some(),
            value.index_health.is_some(),
            value.active_profile.is_some(),
            value.snapshot_present.is_some(),
        ],
    )?;
    validate_repair_progress(value.core.state, value.repair_progress.as_ref())?;
    validate_remediation(
        value.ocr_page_budget_blocked,
        value.ocr_remediation.as_deref(),
        "raise OCR max pages per document or skip oversized scanned PDFs",
    )?;
    validate_remediation(
        value.ocr_language_unavailable,
        value.ocr_language_remediation.as_deref(),
        "install requested OCR language packs or choose an installed OCR language",
    )?;
    if let Some(latency) = &value.query_latency {
        validate_latency(latency.p50_ms, latency.p95_ms, latency.p99_ms)?;
    }
    ensure(value.visible_epoch.is_some() || value.latest_import_scan.is_none())?;
    Ok(value)
}

fn validate_remediation(
    count: Option<SafeCount>,
    remediation: Option<&str>,
    action: &str,
) -> Result<(), DesktopError> {
    match (count.map(SafeCount::value), remediation) {
        (None, None) => Ok(()),
        (Some(0), Some("none")) => Ok(()),
        (Some(value), Some(actual)) if value > 0 && actual == action => Ok(()),
        _ => ensure(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_response::health_contract::CoreState;
    use daemon_contract::CoreReason;

    fn capabilities(state: &str, reason: Option<&str>) -> serde_json::Value {
        let value = serde_json::json!({"state": state, "reason": reason});
        serde_json::json!({
            "keyword_search": value,
            "detail": value,
            "semantic_search": value,
            "hybrid_search": value,
            "text_import": value,
            "ocr_import": value,
            "index_publication": value,
        })
    }

    fn runtimes(state: &str, reason: Option<&str>) -> serde_json::Value {
        let value = serde_json::json!({"state": state, "reason": reason});
        serde_json::json!({"embedding": value, "ocr": value, "classifier": value})
    }

    fn status_payload() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/daemon-status-v3-ready.json"
        ))
        .unwrap()
    }

    #[test]
    fn status_v3_accepts_ready_and_initializing_but_rejects_v2_and_unknown_fields() {
        let ready = status_payload();
        assert!(project_status(&serde_json::to_vec(&ready).unwrap()).is_ok());

        let mut initializing = ready.clone();
        initializing["status"] = serde_json::json!("initializing");
        initializing["core"] =
            serde_json::json!({"state": "initializing", "reason": "metadata_initializing"});
        initializing["optional_runtimes"] = runtimes("initializing", None);
        initializing["capabilities"] = capabilities("initializing", Some("core_initializing"));
        initializing["error"] = serde_json::json!({
            "code": "SERVICE_INITIALIZING",
            "action": "wait_for_service",
            "capability": null,
            "reason": "metadata_initializing"
        });
        for field in [
            "indexed_documents",
            "searchable_documents",
            "partial_documents",
            "visible_epoch",
            "failed_retryable",
            "failed_permanent",
            "recovery_queue_depth",
            "ocr_queue_depth",
            "ocr_jobs_queued",
            "ocr_page_budget_blocked",
            "ocr_remediation",
            "ocr_language_unavailable",
            "ocr_language_remediation",
            "embedding_queue_depth",
            "entity_mentions",
            "import_tasks_queued",
            "import_tasks_recoverable",
            "import_tasks_cancelled",
            "import_scan_scopes",
            "import_scan_errors",
            "query_latency",
            "active_profile",
            "index_health",
            "snapshot_present",
        ] {
            initializing[field] = serde_json::Value::Null;
        }
        assert!(project_status(&serde_json::to_vec(&initializing).unwrap()).is_ok());

        let mut legacy = ready.clone();
        legacy["schema_version"] = serde_json::json!("daemon.status.v2");
        assert!(project_status(&serde_json::to_vec(&legacy).unwrap()).is_err());
        let mut extra = ready;
        extra["private_debug"] = serde_json::json!(true);
        assert!(project_status(&serde_json::to_vec(&extra).unwrap()).is_err());
    }

    #[test]
    fn optional_runtime_degradation_is_independent_from_core_readiness() {
        let mut payload = status_payload();
        payload["optional_runtimes"]["embedding"] =
            serde_json::json!({"state": "unavailable", "reason": "invalid"});
        payload["capabilities"]["semantic_search"] =
            serde_json::json!({"state": "unavailable", "reason": "embedding_unavailable"});
        payload["capabilities"]["hybrid_search"] =
            serde_json::json!({"state": "degraded", "reason": "embedding_unavailable"});
        payload["capabilities"]["text_import"] =
            serde_json::json!({"state": "unavailable", "reason": "embedding_unavailable"});
        payload["capabilities"]["ocr_import"] =
            serde_json::json!({"state": "unavailable", "reason": "embedding_unavailable"});
        payload["capabilities"]["index_publication"] =
            serde_json::json!({"state": "unavailable", "reason": "embedding_unavailable"});
        assert!(project_status(&serde_json::to_vec(&payload).unwrap()).is_ok());

        payload["capabilities"]["text_import"] =
            serde_json::json!({"state": "available", "reason": null});
        assert!(project_status(&serde_json::to_vec(&payload).unwrap()).is_err());
    }

    #[test]
    fn classifier_unavailability_preserves_index_publication() {
        let mut payload = status_payload();
        payload["optional_runtimes"]["classifier"] =
            serde_json::json!({"state": "unavailable", "reason": "not_configured"});
        payload["capabilities"]["text_import"] =
            serde_json::json!({"state": "unavailable", "reason": "classifier_unavailable"});
        payload["capabilities"]["ocr_import"] =
            serde_json::json!({"state": "unavailable", "reason": "classifier_unavailable"});
        assert!(project_status(&serde_json::to_vec(&payload).unwrap()).is_ok());

        payload["capabilities"]["index_publication"] =
            serde_json::json!({"state": "unavailable", "reason": "classifier_unavailable"});
        assert!(project_status(&serde_json::to_vec(&payload).unwrap()).is_err());
    }

    #[test]
    fn artifact_unavailable_blocked_snapshot_matches_the_daemon_contract() {
        let payload = include_bytes!("../../tests/fixtures/daemon-status-v3-artifact-blocked.json");
        let projected = project_status(payload).expect("artifact-blocked status must decode");
        assert!(matches!(projected.core.state, CoreState::Blocked));
        assert!(matches!(
            projected.core.reason,
            Some(CoreReason::ArtifactUnavailable)
        ));
        assert!(projected.visible_epoch.is_none());
    }
}
