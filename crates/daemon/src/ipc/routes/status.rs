use std::net::TcpStream;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportScanProfile, ImportScanScope, MetaStoreErrorClass, ReadMetaStore,
    SearchProjectionServiceState,
};

use super::super::diagnostics;
use super::super::protocol::Request;
use super::super::{
    projection_service_health, repair_progress_json, search_repair_reason_label,
    service_error_json, IpcMetricsSnapshot, ServiceErrorCode, ServiceHealth, ServiceState,
};
use super::{authorized, unauthorized_body, write, RouteResult};

type MetadataReadResult<T> = Result<T, MetaStoreErrorClass>;

pub(super) fn status(store: &ReadMetaStore, stream: &mut TcpStream) -> RouteResult {
    let body = status_json(store);
    write(stream, 200, "application/json", &body)
}

pub(super) fn diagnostics(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let body = diagnostics::render(store);
    write(stream, 200, "application/json", &body)
}

pub(crate) fn query_service_error(store: &ReadMetaStore) -> Option<ServiceErrorCode> {
    match store.search_projection_state() {
        Ok(state) => projection_query_error(Some(state.service_state)),
        Err(_) => projection_query_error(None),
    }
}

pub(crate) fn projection_query_error(
    state: Option<SearchProjectionServiceState>,
) -> Option<ServiceErrorCode> {
    match state {
        Some(SearchProjectionServiceState::Ready) => None,
        Some(SearchProjectionServiceState::Repairing) => Some(ServiceErrorCode::Repairing),
        Some(SearchProjectionServiceState::RepairBlocked) => {
            Some(ServiceErrorCode::QueryServiceRepairRequired)
        }
        None => Some(ServiceErrorCode::MetadataUnavailable),
    }
}

pub(crate) fn status_json(store: &ReadMetaStore) -> String {
    status_json_with(|| status_json_once(store))
}

pub(crate) fn status_json_with(read: impl FnMut() -> MetadataReadResult<String>) -> String {
    retry_metadata_read(read).unwrap_or_else(|_| unavailable_status_json())
}

pub(crate) fn unavailable_status_json() -> String {
    let services = ServiceHealth {
        metadata: ServiceState::Unavailable,
        query: ServiceState::Unavailable,
    };
    let metrics = super::super::process_metrics().snapshot();
    serde_json::json!({
        "schema_version": "daemon.status.v2",
        "status": "degraded",
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "repair_reason": serde_json::Value::Null,
        "repair_progress": serde_json::Value::Null,
        "error": {
            "code": "METADATA_UNAVAILABLE",
            "action": "retry",
        },
        "indexed_documents": serde_json::Value::Null,
        "searchable_documents": serde_json::Value::Null,
        "partial_documents": serde_json::Value::Null,
        "visible_epoch": serde_json::Value::Null,
        "ipc": metrics_json(metrics),
    })
    .to_string()
}

fn status_json_once(store: &ReadMetaStore) -> MetadataReadResult<String> {
    let summary = store.status_summary().map_err(|error| error.class())?;
    let projection = store
        .search_projection_state()
        .map_err(|error| error.class())?;
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(|error| error.class())?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    let services = projection_service_health(projection.service_state);
    let repair_attempt = store
        .artifact_repair_attempt_state()
        .map_err(|error| error.class())?;
    let metrics = super::super::process_metrics().snapshot();
    let body = serde_json::json!({
        "schema_version": "daemon.status.v2",
        "status": match services.aggregate() {
            ServiceState::Ready => "ok",
            ServiceState::Repairing => "repairing",
            ServiceState::Degraded | ServiceState::Unavailable => "degraded",
        },
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "repair_reason": projection.repair_reason.map(search_repair_reason_label),
        "repair_progress": repair_progress_json(
            &projection,
            repair_attempt.as_ref(),
            unix_now_seconds(),
        ),
        "error": service_error_json(services),
        "ipc": metrics_json(metrics),
        "visible_epoch": projection.visible_epoch,
        "indexed_documents": summary.indexed_documents,
        "searchable_documents": summary.searchable_documents,
        "partial_documents": summary.partial_documents,
        "failed_retryable": summary.failed_retryable,
        "failed_permanent": summary.failed_permanent,
        "recovery_queue_depth": summary.recovery_queue_depth,
        "ocr_queue_depth": summary.ocr_queue_depth,
        "ocr_jobs_queued": summary.ocr_jobs_queued,
        "ocr_page_budget_blocked": summary.ocr_page_budget_blocked,
        "ocr_remediation": if summary.ocr_page_budget_blocked > 0 {
            crate::OCR_PAGE_BUDGET_REMEDIATION
        } else {
            "none"
        },
        "ocr_language_unavailable": summary.ocr_language_unavailable,
        "ocr_language_remediation": if summary.ocr_language_unavailable > 0 {
            crate::OCR_LANGUAGE_REMEDIATION
        } else {
            "none"
        },
        "embedding_queue_depth": summary.embedding_queue_depth,
        "entity_mentions": summary.entity_mentions,
        "import_tasks_queued": summary.import_tasks_queued,
        "import_tasks_recoverable": summary.import_tasks_recoverable,
        "import_tasks_cancelled": summary.import_tasks_cancelled,
        "import_scan_scopes": summary.import_scan_scopes,
        "import_scan_errors": summary.import_scan_errors,
        "query_latency": {
            "sample_count": summary.query_latency.sample_count,
            "p50_ms": summary.query_latency.p50_ms,
            "p95_ms": summary.query_latency.p95_ms,
            "p99_ms": summary.query_latency.p99_ms,
            "last_result_count": summary.query_latency.last_result_count,
            "raw_queries": "<redacted>",
        },
        "latest_import_scan": latest_import_scan,
        "active_profile": "balanced",
        "index_health": crate::index_health_label(summary.index_health),
        "snapshot_present": summary.last_snapshot_id.is_some(),
    });
    Ok(body.to_string())
}

fn unix_now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

fn metrics_json(metrics: IpcMetricsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "accepted": metrics.accepted,
        "completed": metrics.completed,
        "client_disconnect": metrics.client_disconnect,
        "request_failure": metrics.request_failure,
        "response_failure": metrics.response_failure,
    })
}

fn retry_metadata_read<T>(
    mut read: impl FnMut() -> MetadataReadResult<T>,
) -> MetadataReadResult<T> {
    let mut last_error = None;
    for attempt in 1..=crate::IPC_METADATA_READ_ATTEMPTS {
        match read() {
            Ok(value) => return Ok(value),
            Err(error)
                if error == MetaStoreErrorClass::Storage
                    && attempt < crate::IPC_METADATA_READ_ATTEMPTS =>
            {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(crate::IPC_METADATA_READ_RETRY_MS));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("metadata read retry loop records a failed attempt"))
}

pub(super) fn import_progress_event_json(store: &ReadMetaStore) -> MetadataReadResult<String> {
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(|error| error.class())?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({
        "schema_version": "daemon.import_progress.v1",
        "event": "snapshot",
        "latest_import_scan": latest_import_scan,
    })
    .to_string())
}

fn latest_import_scan_json(scope: &ImportScanScope) -> serde_json::Value {
    serde_json::json!({
        "scan_profile": match scope.scan_profile {
            ImportScanProfile::Explicit => "explicit",
            ImportScanProfile::Discovery => "discovery",
        },
        "files_discovered": scope.files_discovered,
        "ignored_entries": scope.ignored_entries,
        "scan_errors": scope.scan_errors,
        "searchable_documents": scope.searchable_documents,
        "ocr_required_documents": scope.ocr_required_documents,
        "ocr_jobs_queued": scope.ocr_jobs_queued,
        "failed_documents": scope.failed_documents,
        "deleted_documents": scope.deleted_documents,
        "scan_budget_observed": scope.scan_budget_observed,
        "scan_budget_limit": scope.scan_budget_limit,
        "scan_budget_exhausted": scope.scan_budget_exhausted,
    })
}
