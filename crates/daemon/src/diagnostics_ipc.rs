use meta_store::MetaStore;

use crate::ipc::{process_metrics, IpcMetricsSnapshot, ServiceHealth, ServiceState};
use crate::{projection_service_health, service_error_json};

const MAX_ERROR_BUCKETS: usize = 16;

pub(crate) fn render(store: &MetaStore) -> String {
    render_result(render_available(store))
}

fn render_result<E>(result: std::result::Result<String, E>) -> String {
    result.unwrap_or_else(|_| render_unavailable())
}

fn render_available(store: &MetaStore) -> meta_store::Result<String> {
    let summary = store.status_summary()?;
    let projection = store.search_projection_state()?;
    let visible_epoch = projection.visible_epoch;
    let scan_error_buckets = store
        .import_scan_error_breakdown()?
        .into_iter()
        .take(MAX_ERROR_BUCKETS)
        .map(|bucket| {
            serde_json::json!({
                "class": bucket.kind.label(),
                "operation": bucket.operation.label(),
                "count": bucket.count,
            })
        })
        .collect::<Vec<_>>();
    let services = projection_service_health(projection.service_state);
    let ipc = process_metrics().snapshot();
    let body = serde_json::json!({
        "schema_version": "resume-ir.diagnostics.v3",
        "privacy_boundary": "redacted_local_aggregate",
        "contains_raw_resume_text": false,
        "contains_queries": false,
        "contains_resume_paths": false,
        "contains_candidate_results": false,
        "contains_snippet_text": false,
        "visible_epoch": visible_epoch,
        "evidence_lane": "gui_manual",
        "evidence_status": "unaccepted",
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "error": service_error_json(services),
        "metrics": {
            "ipc": ipc_json(ipc),
            "indexed_documents": summary.indexed_documents,
            "searchable_documents": summary.searchable_documents,
            "partial_documents": summary.partial_documents,
            "ocr_queue_depth": summary.ocr_queue_depth,
            "embedding_queue_depth": summary.embedding_queue_depth,
            "recovery_queue_depth": summary.recovery_queue_depth,
            "import_tasks_queued": summary.import_tasks_queued,
            "import_tasks_recoverable": summary.import_tasks_recoverable,
            "import_tasks_cancelled": summary.import_tasks_cancelled,
            "query_latency": {
                "sample_count": summary.query_latency.sample_count,
                "p50_ms": summary.query_latency.p50_ms,
                "p95_ms": summary.query_latency.p95_ms,
                "p99_ms": summary.query_latency.p99_ms,
                "last_result_count": summary.query_latency.last_result_count,
            },
        },
        "error_counts": {
            "failed_retryable": summary.failed_retryable,
            "failed_permanent": summary.failed_permanent,
            "import_scan_errors": summary.import_scan_errors,
            "ocr_page_budget_blocked": summary.ocr_page_budget_blocked,
            "ocr_language_unavailable": summary.ocr_language_unavailable,
            "scan_error_buckets": scan_error_buckets,
        },
        "benchmark_refs": [],
    });
    Ok(body.to_string())
}

fn render_unavailable() -> String {
    let services = ServiceHealth {
        metadata: ServiceState::Unavailable,
        query: ServiceState::Unavailable,
    };
    serde_json::json!({
        "schema_version": "resume-ir.diagnostics.v3",
        "privacy_boundary": "redacted_local_aggregate",
        "contains_raw_resume_text": false,
        "contains_queries": false,
        "contains_resume_paths": false,
        "contains_candidate_results": false,
        "contains_snippet_text": false,
        "visible_epoch": serde_json::Value::Null,
        "evidence_lane": "gui_manual",
        "evidence_status": "unaccepted",
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "metrics": {
            "ipc": ipc_json(process_metrics().snapshot()),
            "indexed_documents": serde_json::Value::Null,
            "searchable_documents": serde_json::Value::Null,
            "partial_documents": serde_json::Value::Null,
        },
        "error_counts": {
            "scan_error_buckets": [],
        },
        "error": {
            "code": "METADATA_UNAVAILABLE",
            "action": "retry",
        },
        "benchmark_refs": [],
    })
    .to_string()
}

fn ipc_json(metrics: IpcMetricsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "accepted": metrics.accepted,
        "completed": metrics.completed,
        "client_disconnect": metrics.client_disconnect,
        "request_failure": metrics.request_failure,
        "response_failure": metrics.response_failure,
    })
}

#[cfg(test)]
mod tests {
    use super::render_result;

    #[test]
    fn runtime_metadata_read_failure_returns_diagnostics_v3() {
        let body = render_result(std::result::Result::<String, ()>::Err(()));
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(value["schema_version"], "resume-ir.diagnostics.v3");
        assert_eq!(value["process_state"], "ready");
        assert_eq!(value["service_state"], "degraded");
        assert_eq!(value["services"]["metadata"], "unavailable");
        assert_eq!(value["services"]["query"], "unavailable");
        assert_eq!(value["error"]["code"], "METADATA_UNAVAILABLE");
        assert!(value["visible_epoch"].is_null());
        assert!(value["metrics"]["indexed_documents"].is_null());
    }
}
