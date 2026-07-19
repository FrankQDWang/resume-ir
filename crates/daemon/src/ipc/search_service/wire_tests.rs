use super::*;

#[test]
fn request_envelope_requires_bounded_identity_capability_deadline_and_payload() {
    let request = parse_request(
        br#"{"schema_version":"resume-ir.ipc-request.v3","request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{"query":"private"}}"#,
    )
    .unwrap();
    assert_eq!(request.request_id, "request-1");
    assert_eq!(request.payload["query"], "private");
    assert!(request.cancel_token.is_none());

    let cancellable = parse_request(
        br#"{"schema_version":"resume-ir.ipc-request.v3","request_id":"request-2","client_capability":"interactive_gui","deadline_ms":200,"cancel_token":"cancel-2","payload":{}}"#,
    )
    .unwrap();
    assert_eq!(cancellable.cancel_token.as_deref(), Some("cancel-2"));

    for invalid in [
        serde_json::json!({"schema_version":"legacy","request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{}}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"bad id","client_capability":"interactive_gui","deadline_ms":200,"payload":{}}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"untrusted","deadline_ms":200,"payload":{}}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":0,"payload":{}}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":[]}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"cancel_token":"private cancel token","payload":{}}),
        serde_json::json!({"schema_version":REQUEST_SCHEMA_VERSION,"request_id":"request-1","client_capability":"interactive_gui","deadline_ms":200,"payload":{},"legacy_alias":true}),
    ] {
        assert!(parse_request(invalid.to_string().as_bytes()).is_err());
    }
}

#[test]
fn cancel_request_requires_bounded_identity() {
    let request = parse_cancel_request(
        br#"{"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"cancel-token-1"}"#,
    )
    .unwrap();
    assert_eq!(request.request_id, "cancel-command-1");
    assert_eq!(request.cancel_token, "cancel-token-1");
    for invalid in [
        serde_json::json!({"schema_version":"legacy","request_id":"cancel-command-1","cancel_token":"cancel-token-1"}),
        serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"bad id","cancel_token":"cancel-token-1"}),
        serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"bad token"}),
        serde_json::json!({"schema_version":"resume-ir.search-cancel-request.v1","request_id":"cancel-command-1","cancel_token":"cancel-token-1","legacy_alias":true}),
    ] {
        assert!(parse_cancel_request(invalid.to_string().as_bytes()).is_err());
    }
}
