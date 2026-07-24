use daemon_contract::{
    validate_health_contract, CapabilityMatrix, CoreError, CoreHealth, OptionalRuntimeMatrix,
    StatusState,
};
use serde_json::Value;

use super::{has_exact_keys, string};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(crate) fn valid_status(body: &Value) -> bool {
    const KEYS: [&str; 34] = [
        "schema_version",
        "status",
        "process_state",
        "core",
        "optional_runtimes",
        "capabilities",
        "error",
        "repair_progress",
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
        "latest_import_scan",
        "active_profile",
        "index_health",
        "snapshot_present",
        "ipc",
    ];
    if !has_exact_keys(body, &KEYS)
        || string(body, "schema_version") != Some("daemon.status.v3")
        || string(body, "process_state") != Some("ready")
    {
        return false;
    }
    let parsed_health = (
        serde_json::from_value::<StatusState>(body["status"].clone()),
        serde_json::from_value::<CoreHealth>(body["core"].clone()),
        serde_json::from_value::<OptionalRuntimeMatrix>(body["optional_runtimes"].clone()),
        serde_json::from_value::<CapabilityMatrix>(body["capabilities"].clone()),
        serde_json::from_value::<Option<CoreError>>(body["error"].clone()),
    );
    let (Ok(status), Ok(core), Ok(runtimes), Ok(capabilities), Ok(error)) = parsed_health else {
        return false;
    };
    let Some(core_state) = string(&body["core"], "state") else {
        return false;
    };
    if validate_health_contract(status, core, runtimes, capabilities, error).is_err()
        || !valid_repair_progress(core_state, body.get("repair_progress"))
        || !valid_store_projection(body)
        || !valid_ipc_metrics(body.get("ipc"))
    {
        return false;
    }
    true
}

fn valid_repair_progress(core: &str, progress: Option<&Value>) -> bool {
    let Some(progress) = progress else {
        return false;
    };
    if progress.is_null() {
        return core != "repairing";
    }
    if core == "ready"
        || !has_exact_keys(
            progress,
            &[
                "phase",
                "attempt",
                "max_attempts",
                "retry_after_ms",
                "last_error_kind",
            ],
        )
        || !matches!(
            string(progress, "phase"),
            Some(
                "queued"
                    | "migration_rebuild"
                    | "source_unavailable"
                    | "rebuilding"
                    | "retry_wait"
                    | "blocked"
            )
        )
        || !valid_nullable_count(progress.get("attempt"), Some(5))
        || !valid_nullable_exact_count(progress.get("max_attempts"), 5)
        || !valid_nullable_count(progress.get("retry_after_ms"), Some(60_000))
    {
        return false;
    }
    matches!(
        nullable_string(progress, "last_error_kind"),
        Some(None)
            | Some(Some(
                "fulltext_publication_busy"
                    | "fulltext_failure"
                    | "vector_publication_busy"
                    | "vector_failure"
                    | "metadata_failure"
                    | "interrupted"
            ))
    )
}

fn valid_store_projection(body: &Value) -> bool {
    const COUNTS: [&str; 17] = [
        "indexed_documents",
        "searchable_documents",
        "partial_documents",
        "failed_retryable",
        "failed_permanent",
        "recovery_queue_depth",
        "ocr_queue_depth",
        "ocr_jobs_queued",
        "ocr_page_budget_blocked",
        "ocr_language_unavailable",
        "embedding_queue_depth",
        "entity_mentions",
        "import_tasks_queued",
        "import_tasks_recoverable",
        "import_tasks_cancelled",
        "import_scan_scopes",
        "import_scan_errors",
    ];
    let epoch = body.get("visible_epoch");
    if epoch.is_some_and(Value::is_null) {
        return COUNTS
            .into_iter()
            .chain([
                "ocr_remediation",
                "ocr_language_remediation",
                "query_latency",
                "latest_import_scan",
                "active_profile",
                "index_health",
                "snapshot_present",
            ])
            .all(|field| body.get(field).is_some_and(Value::is_null));
    }
    if !valid_count(epoch)
        || !COUNTS.into_iter().all(|field| valid_count(body.get(field)))
        || !valid_remediation(
            body,
            "ocr_page_budget_blocked",
            "ocr_remediation",
            "raise OCR max pages per document or skip oversized scanned PDFs",
        )
        || !valid_remediation(
            body,
            "ocr_language_unavailable",
            "ocr_language_remediation",
            "install requested OCR language packs or choose an installed OCR language",
        )
        || !valid_query_latency(body.get("query_latency"))
        || string(body, "active_profile") != Some("balanced")
        || !matches!(
            string(body, "index_health"),
            Some("empty" | "building" | "ready" | "stale")
        )
        || !body.get("snapshot_present").is_some_and(Value::is_boolean)
    {
        return false;
    }
    body.get("latest_import_scan")
        .is_some_and(|scan| scan.is_null() || valid_latest_import_scan(scan))
}

fn valid_remediation(body: &Value, count: &str, field: &str, action: &str) -> bool {
    let Some(count) = body.get(count).and_then(Value::as_u64) else {
        return false;
    };
    string(body, field) == Some(if count == 0 { "none" } else { action })
}

fn valid_query_latency(value: Option<&Value>) -> bool {
    let Some(value) = value else { return false };
    if !has_exact_keys(
        value,
        &[
            "sample_count",
            "p50_ms",
            "p95_ms",
            "p99_ms",
            "last_result_count",
            "raw_queries",
        ],
    ) || !valid_count(value.get("sample_count"))
        || string(value, "raw_queries") != Some("<redacted>")
        || !valid_nullable_count(value.get("last_result_count"), None)
    {
        return false;
    }
    ["p50_ms", "p95_ms", "p99_ms"].into_iter().all(|field| {
        value.get(field).is_some_and(|number| {
            number.is_null()
                || number.as_f64().is_some_and(|number| {
                    number.is_finite() && (0.0..=3_600_000.0).contains(&number)
                })
        })
    })
}

fn valid_latest_import_scan(value: &Value) -> bool {
    const KEYS: [&str; 12] = [
        "scan_profile",
        "files_discovered",
        "ignored_entries",
        "scan_errors",
        "searchable_documents",
        "ocr_required_documents",
        "ocr_jobs_queued",
        "failed_documents",
        "deleted_documents",
        "scan_budget_observed",
        "scan_budget_limit",
        "scan_budget_exhausted",
    ];
    has_exact_keys(value, &KEYS)
        && matches!(
            string(value, "scan_profile"),
            Some("explicit" | "discovery")
        )
        && KEYS[1..9]
            .iter()
            .all(|field| valid_count(value.get(*field)))
        && valid_nullable_count(value.get("scan_budget_observed"), None)
        && valid_nullable_count(value.get("scan_budget_limit"), None)
        && value
            .get("scan_budget_exhausted")
            .is_some_and(Value::is_boolean)
}

fn valid_ipc_metrics(value: Option<&Value>) -> bool {
    let Some(value) = value else { return false };
    const KEYS: [&str; 5] = [
        "accepted",
        "completed",
        "client_disconnect",
        "request_failure",
        "response_failure",
    ];
    has_exact_keys(value, &KEYS) && KEYS.into_iter().all(|field| valid_count(value.get(field)))
}

fn valid_count(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_u64)
        .is_some_and(|value| value <= MAX_SAFE_INTEGER)
}

fn valid_nullable_count(value: Option<&Value>, max: Option<u64>) -> bool {
    value.is_some_and(|value| {
        value.is_null()
            || value.as_u64().is_some_and(|value| {
                value <= MAX_SAFE_INTEGER && max.is_none_or(|max| value <= max)
            })
    })
}

fn valid_nullable_exact_count(value: Option<&Value>, expected: u64) -> bool {
    value.is_some_and(|value| value.is_null() || value.as_u64() == Some(expected))
}

fn nullable_string<'a>(value: &'a Value, field: &str) -> Option<Option<&'a str>> {
    value.get(field).and_then(|value| {
        if value.is_null() {
            Some(None)
        } else {
            value.as_str().map(Some)
        }
    })
}
