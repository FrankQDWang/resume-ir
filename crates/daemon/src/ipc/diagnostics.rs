use meta_store::ReadMetaStore;

use super::{
    process_metrics, repair_progress_json, CapabilityMatrix, CoreHealth, OptionalRuntimeMatrix,
};

const MAX_ERROR_BUCKETS: usize = 16;

fn render_available(
    store: &ReadMetaStore,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> meta_store::Result<serde_json::Value> {
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
    let repair_attempt = store.artifact_repair_attempt_state()?;
    let ipc = process_metrics().snapshot();
    let mut body = serde_json::json!({
        "schema_version": "resume-ir.diagnostics.v4",
        "privacy_boundary": "redacted_local_aggregate",
        "contains_raw_resume_text": false,
        "contains_queries": false,
        "contains_resume_paths": false,
        "contains_candidate_results": false,
        "contains_snippet_text": false,
        "visible_epoch": visible_epoch,
        "evidence_lane": "gui_manual",
        "evidence_status": "unaccepted",
        "repair_progress": repair_progress_json(
            &projection,
            repair_attempt.as_ref(),
            current_unix_seconds(),
        ),
        "error": super::capability::service_error_json(core),
        "metrics": {
            "ipc": ipc.to_json(),
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
    merge_health(&mut body, core, runtimes, capabilities);
    Ok(body)
}

pub(crate) fn render_without_store(
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "schema_version": "resume-ir.diagnostics.v4",
        "privacy_boundary": "redacted_local_aggregate",
        "contains_raw_resume_text": false,
        "contains_queries": false,
        "contains_resume_paths": false,
        "contains_candidate_results": false,
        "contains_snippet_text": false,
        "visible_epoch": serde_json::Value::Null,
        "evidence_lane": "gui_manual",
        "evidence_status": "unaccepted",
        "repair_progress": serde_json::Value::Null,
        "metrics": {
            "ipc": process_metrics().snapshot().to_json(),
            "indexed_documents": serde_json::Value::Null,
            "searchable_documents": serde_json::Value::Null,
            "partial_documents": serde_json::Value::Null,
            "ocr_queue_depth": serde_json::Value::Null,
            "embedding_queue_depth": serde_json::Value::Null,
            "recovery_queue_depth": serde_json::Value::Null,
            "import_tasks_queued": serde_json::Value::Null,
            "import_tasks_recoverable": serde_json::Value::Null,
            "import_tasks_cancelled": serde_json::Value::Null,
            "query_latency": {
                "sample_count": serde_json::Value::Null,
                "p50_ms": serde_json::Value::Null,
                "p95_ms": serde_json::Value::Null,
                "p99_ms": serde_json::Value::Null,
                "last_result_count": serde_json::Value::Null,
            },
        },
        "error_counts": {
            "failed_retryable": serde_json::Value::Null,
            "failed_permanent": serde_json::Value::Null,
            "import_scan_errors": serde_json::Value::Null,
            "ocr_page_budget_blocked": serde_json::Value::Null,
            "ocr_language_unavailable": serde_json::Value::Null,
            "scan_error_buckets": [],
        },
        "error": super::capability::service_error_json(core),
        "benchmark_refs": [],
    });
    merge_health(&mut body, core, runtimes, capabilities);
    body
}

pub(crate) fn render_from_store(
    store: &ReadMetaStore,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) -> meta_store::Result<serde_json::Value> {
    render_available(store, core, runtimes, capabilities)
}

fn merge_health(
    body: &mut serde_json::Value,
    core: CoreHealth,
    runtimes: OptionalRuntimeMatrix,
    capabilities: CapabilityMatrix,
) {
    let health = super::capability::health_json(core, runtimes, capabilities);
    let object = body.as_object_mut().expect("diagnostics body is an object");
    for (key, value) in health.as_object().expect("health body is an object") {
        object.insert(key.clone(), value.clone());
    }
}

fn current_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::render_without_store;
    use crate::ipc::{
        CapabilityMatrix, CoreHealth, CoreReason, CoreState, OptionalRuntimeHealth,
        OptionalRuntimeMatrix, OptionalRuntimeReason,
    };

    #[test]
    fn metadata_unavailable_diagnostics_preserve_the_runtime_matrix() {
        let core = CoreHealth {
            state: CoreState::Degraded,
            reason: Some(CoreReason::MetadataUnavailable),
        };
        let runtimes = OptionalRuntimeMatrix {
            embedding: OptionalRuntimeHealth::available(),
            ocr: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Invalid),
            classifier: OptionalRuntimeHealth::unavailable(OptionalRuntimeReason::Missing),
        };
        let value = render_without_store(core, runtimes, CapabilityMatrix::derive(core, runtimes));

        assert_eq!(value["schema_version"], "resume-ir.diagnostics.v4");
        assert_eq!(value["process_state"], "ready");
        assert_eq!(value["core"]["state"], "degraded");
        assert_eq!(value["core"]["reason"], "metadata_unavailable");
        assert_eq!(value["error"]["code"], "SERVICE_BLOCKED");
        assert_eq!(
            value["optional_runtimes"]["embedding"]["state"],
            "available"
        );
        assert_eq!(value["optional_runtimes"]["ocr"]["reason"], "invalid");
        assert_eq!(
            value["optional_runtimes"]["classifier"]["reason"],
            "missing"
        );
        assert!(value["visible_epoch"].is_null());
        assert!(value["metrics"]["indexed_documents"].is_null());
    }

    #[test]
    fn diagnostics_v4_shared_fixture_matches_producer_shape() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src-tauri/tests/fixtures/daemon-diagnostics-v4-ready.json"
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
        let produced =
            render_without_store(core, runtimes, CapabilityMatrix::derive(core, runtimes));

        assert_eq!(keys(&produced), keys(&fixture));
        for key in [
            "core",
            "optional_runtimes",
            "capabilities",
            "metrics",
            "error_counts",
        ] {
            assert_eq!(keys(&produced[key]), keys(&fixture[key]), "{key}");
        }
        assert_eq!(
            keys(&produced["metrics"]["ipc"]),
            keys(&fixture["metrics"]["ipc"])
        );
        assert_eq!(
            keys(&produced["metrics"]["query_latency"]),
            keys(&fixture["metrics"]["query_latency"])
        );
    }

    fn keys(value: &serde_json::Value) -> BTreeSet<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }
}
