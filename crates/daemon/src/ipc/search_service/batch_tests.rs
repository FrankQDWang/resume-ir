use super::*;

fn child(request_id: &str, capability: &str, cancel_token: Option<&str>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": capability,
        "deadline_ms": 200,
        "payload": {"query": "synthetic"},
    });
    if let Some(cancel_token) = cancel_token {
        value["cancel_token"] = serde_json::json!(cancel_token);
    }
    value
}

#[test]
fn batch_request_requires_bounded_homogeneous_unique_children() {
    let valid = serde_json::json!({
        "schema_version": REQUEST_SCHEMA_VERSION,
        "batch_id": "batch-1",
        "requests": [
            child("request-1", "benchmark", Some("cancel-1")),
            child("request-2", "benchmark", Some("cancel-2")),
        ],
    });
    let parsed = parse_request(valid.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.batch_id, "batch-1");
    assert_eq!(parsed.requests.len(), 2);

    for invalid in [
        serde_json::json!({"schema_version": "legacy", "batch_id": "batch-1", "requests": [child("request-1", "benchmark", None)]}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "bad id", "requests": [child("request-1", "benchmark", None)]}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "batch-1", "requests": []}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "batch-1", "requests": [child("same", "benchmark", None), child("same", "benchmark", None)]}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "batch-1", "requests": [child("request-1", "benchmark", Some("same")), child("request-2", "benchmark", Some("same"))]}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "batch-1", "requests": [child("request-1", "benchmark", None), child("request-2", "interactive_gui", None)]}),
        serde_json::json!({"schema_version": REQUEST_SCHEMA_VERSION, "batch_id": "batch-1", "requests": [child("request-1", "benchmark", None)], "legacy_alias": true}),
    ] {
        assert!(parse_request(invalid.to_string().as_bytes()).is_err());
    }
}

#[test]
fn batch_request_rejects_more_than_sixty_four_children() {
    let requests = (0..65)
        .map(|index| child(&format!("request-{index}"), "benchmark", None))
        .collect::<Vec<_>>();
    let invalid = serde_json::json!({
        "schema_version": REQUEST_SCHEMA_VERSION,
        "batch_id": "batch-1",
        "requests": requests,
    });
    assert!(parse_request(invalid.to_string().as_bytes()).is_err());
}

#[test]
fn batch_admission_failure_uses_the_unified_error_v2_contract() {
    let value: serde_json::Value = serde_json::from_str(&overload_body("batch-1")).unwrap();

    assert_eq!(value["schema_version"], "resume-ir.error.v2");
    assert_eq!(value["request_id"], "batch-1");
    assert_eq!(value["error"]["code"], "OVERLOADED");
    assert_eq!(value["error"]["action"], "retry");
    assert_eq!(value["error"]["retry_after_ms"], 250);
    assert!(value.get("batch_id").is_none());
}
