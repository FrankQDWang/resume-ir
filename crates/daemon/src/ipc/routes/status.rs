use std::net::TcpStream;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportScanProfile, ImportScanScope, MetaStoreErrorClass, ReadMetaStore,
    SearchProjectionServiceState,
};

use super::super::protocol::Request;
use super::super::{
    repair_progress_json, CapabilityMatrix, ControlPlaneState, CoreHealth, CoreState,
    OptionalRuntimeMatrix, ServiceErrorCode,
};
#[cfg(test)]
use super::super::{CoreReason, OptionalRuntimeHealth, OptionalRuntimeReason};
use super::{authorized, unauthorized_body, write, RouteResult};

type MetadataReadResult<T> = Result<T, MetaStoreErrorClass>;

pub(super) fn status(
    state: &ControlPlaneState,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let body = state.status_body();
    write(stream, 200, "application/json", &body)
}

pub(super) fn diagnostics(
    state: &ControlPlaneState,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let body = state.diagnostics_body();
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
            Some(ServiceErrorCode::QueryServiceUnavailable)
        }
        None => Some(ServiceErrorCode::MetadataUnavailable),
    }
}

#[cfg(test)]
pub(crate) fn status_json_with(read: impl FnMut() -> MetadataReadResult<String>) -> String {
    retry_metadata_read(read).unwrap_or_else(|_| unavailable_status_json())
}

#[cfg(test)]
pub(crate) fn unavailable_status_json() -> String {
    let core = CoreHealth {
        state: CoreState::Degraded,
        reason: Some(CoreReason::MetadataUnavailable),
    };
    let runtimes = unavailable_runtimes(OptionalRuntimeReason::NotConfigured);
    render_without_store(core, runtimes, CapabilityMatrix::derive(core, runtimes)).to_string()
}

pub(crate) fn render_without_store(
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> serde_json::Value {
    let metrics = super::super::process_metrics().snapshot();
    let mut body = serde_json::json!({
        "schema_version": "daemon.status.v3",
        "status": status_label(core.state),
        "error": super::super::capability::service_error_json(core),
        "repair_progress": serde_json::Value::Null,
        "indexed_documents": serde_json::Value::Null,
        "searchable_documents": serde_json::Value::Null,
        "partial_documents": serde_json::Value::Null,
        "visible_epoch": serde_json::Value::Null,
        "failed_retryable": serde_json::Value::Null,
        "failed_permanent": serde_json::Value::Null,
        "recovery_queue_depth": serde_json::Value::Null,
        "ocr_queue_depth": serde_json::Value::Null,
        "ocr_jobs_queued": serde_json::Value::Null,
        "ocr_page_budget_blocked": serde_json::Value::Null,
        "ocr_remediation": serde_json::Value::Null,
        "ocr_language_unavailable": serde_json::Value::Null,
        "ocr_language_remediation": serde_json::Value::Null,
        "embedding_queue_depth": serde_json::Value::Null,
        "entity_mentions": serde_json::Value::Null,
        "import_tasks_queued": serde_json::Value::Null,
        "import_tasks_recoverable": serde_json::Value::Null,
        "import_tasks_cancelled": serde_json::Value::Null,
        "import_scan_scopes": serde_json::Value::Null,
        "import_scan_errors": serde_json::Value::Null,
        "query_latency": serde_json::Value::Null,
        "latest_import_scan": serde_json::Value::Null,
        "active_profile": serde_json::Value::Null,
        "index_health": serde_json::Value::Null,
        "snapshot_present": serde_json::Value::Null,
        "ipc": metrics.to_json(),
    });
    merge_health(&mut body, core, runtimes, capabilities);
    body
}

pub(crate) fn render_from_store(
    store: &ReadMetaStore,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> MetadataReadResult<serde_json::Value> {
    retry_metadata_read(|| status_json_once(store, core, runtimes, capabilities))
}

fn status_json_once(
    store: &ReadMetaStore,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> MetadataReadResult<serde_json::Value> {
    let summary = store.status_summary().map_err(|error| error.class())?;
    let projection = store
        .search_projection_state()
        .map_err(|error| error.class())?;
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(|error| error.class())?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    let repair_attempt = store
        .artifact_repair_attempt_state()
        .map_err(|error| error.class())?;
    let metrics = super::super::process_metrics().snapshot();
    let mut body = serde_json::json!({
        "schema_version": "daemon.status.v3",
        "status": status_label(core.state),
        "repair_progress": repair_progress_json(
            &projection,
            repair_attempt.as_ref(),
            unix_now_seconds(),
        ),
        "error": super::super::capability::service_error_json(core),
        "ipc": metrics.to_json(),
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
    merge_health(&mut body, core, runtimes, capabilities);
    Ok(body)
}

fn merge_health(
    body: &mut serde_json::Value,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) {
    let health = super::super::capability::health_json(core, runtimes, capabilities);
    let object = body.as_object_mut().expect("status body is an object");
    for (key, value) in health.as_object().expect("health body is an object") {
        object.insert(key.clone(), value.clone());
    }
}

fn status_label(state: CoreState) -> &'static str {
    match state {
        CoreState::Initializing => "initializing",
        CoreState::Ready => "ok",
        CoreState::Repairing => "repairing",
        CoreState::Degraded => "degraded",
        CoreState::Blocked => "blocked",
    }
}

#[cfg(test)]
fn unavailable_runtimes(reason: OptionalRuntimeReason) -> OptionalRuntimeMatrix {
    OptionalRuntimeMatrix {
        embedding: OptionalRuntimeHealth::unavailable(reason),
        ocr: OptionalRuntimeHealth::unavailable(reason),
        classifier: OptionalRuntimeHealth::unavailable(reason),
    }
}

fn unix_now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
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

#[cfg(test)]
mod contract_tests {
    use std::collections::BTreeSet;

    use super::render_without_store;
    use crate::ipc::{
        CapabilityMatrix, CoreHealth, CoreState, OptionalRuntimeHealth, OptionalRuntimeMatrix,
    };

    #[test]
    fn daemon_status_v3_ready_fixture_matches_producer_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src-tauri/tests/fixtures/daemon-status-v3-ready.json"
        )))
        .unwrap();
        let core = CoreHealth {
            state: CoreState::Ready,
            reason: None,
        };
        let runtimes = OptionalRuntimeMatrix {
            embedding: OptionalRuntimeHealth::available(),
            ocr: OptionalRuntimeHealth::available(),
            classifier: OptionalRuntimeHealth::available(),
        };
        let capabilities = CapabilityMatrix::derive(core, runtimes);
        let rendered = render_without_store(core, runtimes, capabilities);

        assert_eq!(object_keys(&rendered), object_keys(&fixture));
        for key in [
            "schema_version",
            "status",
            "process_state",
            "core",
            "optional_runtimes",
            "capabilities",
            "error",
        ] {
            assert_eq!(rendered[key], fixture[key], "contract drift at {key}");
        }
        assert_eq!(
            object_keys(&rendered["optional_runtimes"]),
            object_keys(&fixture["optional_runtimes"])
        );
        assert_eq!(
            object_keys(&rendered["capabilities"]),
            object_keys(&fixture["capabilities"])
        );
    }

    fn object_keys(value: &serde_json::Value) -> BTreeSet<&str> {
        value
            .as_object()
            .expect("contract node is an object")
            .keys()
            .map(String::as_str)
            .collect()
    }
}
