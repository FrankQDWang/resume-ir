use super::error::ImportServiceError;
use super::*;

fn ready_status() -> Value {
    serde_json::json!({
        "schema_version": "daemon.status.v3",
        "status": "ok",
        "process_state": "ready",
        "core": {"state": "ready", "reason": null},
        "optional_runtimes": {
            "embedding": {"state": "available", "reason": null},
            "ocr": {"state": "available", "reason": null},
            "classifier": {"state": "available", "reason": null}
        },
        "capabilities": {
            "keyword_search": {"state": "available", "reason": null},
            "detail": {"state": "available", "reason": null},
            "semantic_search": {"state": "available", "reason": null},
            "hybrid_search": {"state": "available", "reason": null},
            "text_import": {"state": "available", "reason": null},
            "ocr_import": {"state": "available", "reason": null},
            "index_publication": {"state": "available", "reason": null}
        },
        "error": null,
        "repair_progress": null,
        "indexed_documents": 4,
        "searchable_documents": 3,
        "partial_documents": 1,
        "visible_epoch": 7,
        "failed_retryable": 0,
        "failed_permanent": 0,
        "recovery_queue_depth": 0,
        "ocr_queue_depth": 0,
        "ocr_jobs_queued": 0,
        "ocr_page_budget_blocked": 0,
        "ocr_remediation": "none",
        "ocr_language_unavailable": 0,
        "ocr_language_remediation": "none",
        "embedding_queue_depth": 0,
        "entity_mentions": 8,
        "import_tasks_queued": 0,
        "import_tasks_recoverable": 0,
        "import_tasks_cancelled": 0,
        "import_scan_scopes": 1,
        "import_scan_errors": 0,
        "query_latency": {
            "sample_count": 1,
            "p50_ms": 2.0,
            "p95_ms": 3.0,
            "p99_ms": 4.0,
            "last_result_count": 1,
            "raw_queries": "<redacted>"
        },
        "latest_import_scan": null,
        "active_profile": "balanced",
        "index_health": "ready",
        "snapshot_present": true,
        "ipc": {"accepted": 2, "completed": 2, "client_disconnect": 0, "request_failure": 0, "response_failure": 0}
    })
}

#[test]
fn discovery_and_auth_require_exact_v3_launch_and_instance_binding() {
    let launch = "a".repeat(64);
    let instance = "b".repeat(64);
    let token = "c".repeat(64);
    let addr = "127.0.0.1:4123";
    let discovery = serde_json::json!({
        "schema_version": DISCOVERY_SCHEMA,
        "launch_id": launch,
        "instance_id": instance,
        "owner_mode": "standalone",
        "status": format!("http://{addr}/status"),
        "diagnostics": format!("http://{addr}/diagnostics"),
        "imports": format!("http://{addr}/imports"),
        "import_cancel": format!("http://{addr}/imports/cancel"),
        "import_control": format!("http://{addr}/imports/control"),
        "import_progress": format!("http://{addr}/imports/progress"),
        "search": format!("http://{addr}/search"),
        "search_batch": format!("http://{addr}/search/batch"),
        "details": format!("http://{addr}/details"),
        "delete": format!("http://{addr}/delete")
    });
    let auth = serde_json::json!({
        "schema_version": AUTH_SCHEMA,
        "launch_id": launch,
        "instance_id": instance,
        "token": token
    });
    let bound = parse_discovery(&discovery.to_string())
        .unwrap()
        .bind(parse_auth(&auth.to_string()).unwrap())
        .unwrap();
    assert_eq!(bound.addr().to_string(), addr);
    assert_eq!(bound.token(), "c".repeat(64));

    let mut legacy = discovery.clone();
    legacy["schema_version"] = Value::String("resume-ir.daemon-ipc.v2".to_string());
    assert!(parse_discovery(&legacy.to_string()).is_none());
    let mut unknown_discovery = discovery.clone();
    unknown_discovery["private_debug"] = Value::Bool(true);
    assert!(parse_discovery(&unknown_discovery.to_string()).is_none());

    let mut legacy_auth = auth.clone();
    legacy_auth["schema_version"] = Value::String("resume-ir.daemon-auth.v2".to_string());
    assert!(parse_auth(&legacy_auth.to_string()).is_none());
    let mut unknown_auth = auth.clone();
    unknown_auth["private_debug"] = Value::Bool(true);
    assert!(parse_auth(&unknown_auth.to_string()).is_none());

    let mut wrong_launch = auth.clone();
    wrong_launch["launch_id"] = Value::String("d".repeat(64));
    assert!(parse_discovery(&discovery.to_string())
        .unwrap()
        .bind(parse_auth(&wrong_launch.to_string()).unwrap())
        .is_none());
    let mut wrong_instance = auth;
    wrong_instance["instance_id"] = Value::String("e".repeat(64));
    assert!(parse_discovery(&discovery.to_string())
        .unwrap()
        .bind(parse_auth(&wrong_instance.to_string()).unwrap())
        .is_none());
}

#[test]
fn status_v3_rejects_old_unknown_and_illegal_state_combinations() {
    let ready = ready_status();
    assert!(valid_status(&ready));

    let mut legacy = ready.clone();
    legacy["schema_version"] = Value::String("daemon.status.v2".to_string());
    assert!(!valid_status(&legacy));
    let mut unknown = ready.clone();
    unknown["private_debug"] = Value::Bool(true);
    assert!(!valid_status(&unknown));
    let mut illegal = ready;
    illegal["core"] = serde_json::json!({"state": "blocked", "reason": "runtime_invariant"});
    assert!(!valid_status(&illegal));
}

#[test]
fn status_v3_accepts_initializing_with_null_store_projection() {
    let mut status = ready_status();
    status["status"] = Value::String("initializing".to_string());
    status["core"] =
        serde_json::json!({"state": "initializing", "reason": "metadata_initializing"});
    status["optional_runtimes"] = serde_json::json!({
        "embedding": {"state": "initializing", "reason": null},
        "ocr": {"state": "initializing", "reason": null},
        "classifier": {"state": "initializing", "reason": null}
    });
    for capability in [
        "keyword_search",
        "detail",
        "semantic_search",
        "hybrid_search",
        "text_import",
        "ocr_import",
        "index_publication",
    ] {
        status["capabilities"][capability] =
            serde_json::json!({"state": "initializing", "reason": "core_initializing"});
    }
    status["error"] = serde_json::json!({
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
        "latest_import_scan",
        "active_profile",
        "index_health",
        "snapshot_present",
    ] {
        status[field] = Value::Null;
    }
    assert!(valid_status(&status));
}

#[test]
fn status_v3_accepts_independent_capability_reasons_for_combined_runtime_failure() {
    let mut status = ready_status();
    status["optional_runtimes"] = serde_json::json!({
        "embedding": {"state": "unavailable", "reason": "not_configured"},
        "ocr": {"state": "unavailable", "reason": "not_configured"},
        "classifier": {"state": "unavailable", "reason": "not_configured"}
    });
    status["capabilities"] = serde_json::json!({
        "keyword_search": {"state": "available", "reason": null},
        "detail": {"state": "available", "reason": null},
        "semantic_search": {"state": "unavailable", "reason": "embedding_unavailable"},
        "hybrid_search": {"state": "degraded", "reason": "embedding_unavailable"},
        "text_import": {"state": "unavailable", "reason": "classifier_unavailable"},
        "ocr_import": {"state": "unavailable", "reason": "classifier_unavailable"},
        "index_publication": {"state": "unavailable", "reason": "embedding_unavailable"}
    });

    assert!(valid_status(&status));
}

#[test]
fn import_service_error_v2_accepts_only_exact_import_context() {
    let capability_unavailable = serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": "CAPABILITY_UNAVAILABLE",
            "action": "select_supported_mode",
            "capability": "text_import",
            "reason": "classifier_unavailable"
        }
    });
    assert_eq!(
        parse_import_service_error(&capability_unavailable.to_string(), 503),
        Some(ImportServiceError::CapabilityUnavailable)
    );

    let initializing = serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": "SERVICE_INITIALIZING",
            "action": "wait_for_service",
            "capability": null,
            "reason": "migration_rebuild"
        }
    });
    assert_eq!(
        parse_import_service_error(&initializing.to_string(), 503),
        Some(ImportServiceError::Initializing)
    );

    let blocked = serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": "SERVICE_BLOCKED",
            "action": "repair_required",
            "capability": null,
            "reason": "unsupported_store_schema"
        }
    });
    assert_eq!(
        parse_import_service_error(&blocked.to_string(), 503),
        Some(ImportServiceError::Blocked)
    );

    let mut unknown = capability_unavailable;
    unknown["error"]["unexpected"] = Value::Bool(true);
    assert_eq!(parse_import_service_error(&unknown.to_string(), 503), None);
    assert_eq!(parse_import_service_error(&blocked.to_string(), 500), None);
}
