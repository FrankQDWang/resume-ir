use serde::{Deserialize, Serialize};

use super::enums::{DaemonErrorStatus, ErrorStatus};
use super::{decode, ensure, ensure_schema, protocol_error};
use crate::daemon_client::DesktopError;
use crate::daemon_exchange::{valid_opaque_id, ExpectedResponse};

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ErrorBody {
    Unified(UnifiedErrorBody),
    Daemon(DaemonErrorBody),
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    schema_version: String,
}

#[derive(Deserialize, Serialize)]
pub(super) struct DaemonErrorBody {
    schema_version: String,
    status: DaemonErrorStatus,
}

#[derive(Deserialize, Serialize)]
pub(super) struct UnifiedErrorBody {
    schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    status: ErrorStatus,
    error: UnifiedError,
}

#[derive(Deserialize, Serialize)]
struct UnifiedError {
    code: UnifiedErrorCode,
    action: UnifiedErrorAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum UnifiedErrorCode {
    Unauthorized,
    BadRequest,
    Conflict,
    NotFound,
    StaleSelection,
    ResponseTooLarge,
    LimitExceeded,
    Repairing,
    MetadataUnavailable,
    QueryServiceUnavailable,
    SemanticDisabled,
    Overloaded,
    Internal,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UnifiedErrorAction {
    Authenticate,
    CorrectRequest,
    RefreshSearch,
    ReducePageSize,
    WaitForRepair,
    Retry,
    SelectSupportedMode,
}

pub(super) fn project_error(
    body: &[u8],
    http_status: u16,
    expected: &ExpectedResponse,
) -> Result<ErrorBody, DesktopError> {
    let envelope: ErrorEnvelope = decode(body)?;
    match envelope.schema_version.as_str() {
        "resume-ir.error.v1" => {
            project_unified_error(body, http_status, expected).map(ErrorBody::Unified)
        }
        "daemon.error.v1" => project_daemon_error(body, http_status).map(ErrorBody::Daemon),
        _ => Err(protocol_error()),
    }
}

fn project_unified_error(
    body: &[u8],
    http_status: u16,
    expected: &ExpectedResponse,
) -> Result<UnifiedErrorBody, DesktopError> {
    let value: UnifiedErrorBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.error.v1")?;
    ensure(error_http_status(&value.error.code) == http_status)?;
    ensure(action_matches(&value.error.code, &value.error.action))?;
    ensure(code_allowed_for_response(&value.error.code, expected))?;
    ensure(match value.error.code {
        UnifiedErrorCode::Overloaded => value
            .error
            .retry_after_ms
            .is_some_and(|milliseconds| (1..=60_000).contains(&milliseconds)),
        _ => value.error.retry_after_ms.is_none(),
    })?;
    match expected {
        ExpectedResponse::Search { request_id, .. }
        | ExpectedResponse::Detail { request_id, .. }
        | ExpectedResponse::Hydrate { request_id, .. } => ensure(
            value.request_id.as_deref() == Some(request_id.as_str()) && valid_opaque_id(request_id),
        )?,
        _ => ensure(value.request_id.is_none())?,
    }
    Ok(value)
}

fn project_daemon_error(body: &[u8], http_status: u16) -> Result<DaemonErrorBody, DesktopError> {
    let value: DaemonErrorBody = decode(body)?;
    ensure_schema(&value.schema_version, "daemon.error.v1")?;
    ensure(daemon_error_http_status(&value.status) == http_status)?;
    Ok(value)
}

fn error_http_status(code: &UnifiedErrorCode) -> u16 {
    match code {
        UnifiedErrorCode::Unauthorized => 401,
        UnifiedErrorCode::BadRequest => 400,
        UnifiedErrorCode::NotFound => 404,
        UnifiedErrorCode::Conflict | UnifiedErrorCode::StaleSelection => 409,
        UnifiedErrorCode::ResponseTooLarge | UnifiedErrorCode::LimitExceeded => 413,
        UnifiedErrorCode::Repairing
        | UnifiedErrorCode::MetadataUnavailable
        | UnifiedErrorCode::QueryServiceUnavailable
        | UnifiedErrorCode::SemanticDisabled
        | UnifiedErrorCode::Overloaded => 503,
        UnifiedErrorCode::Internal => 500,
    }
}

fn action_matches(code: &UnifiedErrorCode, action: &UnifiedErrorAction) -> bool {
    matches!(
        (code, action),
        (
            UnifiedErrorCode::Unauthorized,
            UnifiedErrorAction::Authenticate
        ) | (
            UnifiedErrorCode::BadRequest,
            UnifiedErrorAction::CorrectRequest
        ) | (
            UnifiedErrorCode::NotFound | UnifiedErrorCode::StaleSelection,
            UnifiedErrorAction::RefreshSearch
        ) | (
            UnifiedErrorCode::ResponseTooLarge | UnifiedErrorCode::LimitExceeded,
            UnifiedErrorAction::ReducePageSize
        ) | (
            UnifiedErrorCode::Repairing,
            UnifiedErrorAction::WaitForRepair
        ) | (
            UnifiedErrorCode::SemanticDisabled,
            UnifiedErrorAction::SelectSupportedMode
        ) | (
            UnifiedErrorCode::Conflict
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
                | UnifiedErrorCode::Overloaded
                | UnifiedErrorCode::Internal,
            UnifiedErrorAction::Retry
        )
    )
}

fn code_allowed_for_response(code: &UnifiedErrorCode, expected: &ExpectedResponse) -> bool {
    match expected {
        ExpectedResponse::Search { .. } => matches!(
            code,
            UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::Conflict
                | UnifiedErrorCode::NotFound
                | UnifiedErrorCode::LimitExceeded
                | UnifiedErrorCode::Repairing
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
                | UnifiedErrorCode::SemanticDisabled
                | UnifiedErrorCode::Overloaded
                | UnifiedErrorCode::Internal
        ),
        ExpectedResponse::Detail { .. } | ExpectedResponse::Hydrate { .. } => matches!(
            code,
            UnifiedErrorCode::Unauthorized
                | UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::NotFound
                | UnifiedErrorCode::StaleSelection
                | UnifiedErrorCode::ResponseTooLarge
                | UnifiedErrorCode::Repairing
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
        ),
        _ => matches!(
            code,
            UnifiedErrorCode::Unauthorized
                | UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::Repairing
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
        ),
    }
}

fn daemon_error_http_status(status: &DaemonErrorStatus) -> u16 {
    match status {
        DaemonErrorStatus::Unauthorized => 401,
        DaemonErrorStatus::BadRequest => 400,
        DaemonErrorStatus::Conflict => 409,
        DaemonErrorStatus::NotFound => 404,
        DaemonErrorStatus::TooLarge => 413,
        DaemonErrorStatus::Internal => 500,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_exchange::SearchSelection;

    fn selection() -> SearchSelection {
        serde_json::from_value(serde_json::json!({
            "doc_id": "doc_00000000000000000000000000000000",
            "version_id": "ver_00000000000000000000000000000000",
            "visible_epoch": 7,
        }))
        .unwrap()
    }

    #[test]
    fn detail_errors_are_fixed_bounded_and_never_expose_selection() {
        let expected = ExpectedResponse::Detail {
            request_id: "detail-request".to_string(),
            selection: selection(),
        };
        let stale = br#"{"schema_version":"resume-ir.error.v1","request_id":"detail-request","status":"error","error":{"code":"STALE_SELECTION","action":"refresh_search","selection":{"doc_id":"doc_private","version_id":"ver_private"}},"private_debug":true}"#;
        let projected = project_error(stale, 409, &expected).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(exposed.contains("STALE_SELECTION"));
        assert!(!exposed.contains("doc_private"));
        assert!(!exposed.contains("ver_private"));
        assert!(!exposed.contains("selection"));

        let wrong_action = br#"{"schema_version":"resume-ir.error.v1","request_id":"detail-request","status":"error","error":{"code":"STALE_SELECTION","action":"retry"}}"#;
        assert!(project_error(wrong_action, 409, &expected).is_err());
        assert!(project_error(stale, 404, &expected).is_err());
        assert!(project_error(stale, 409, &ExpectedResponse::Status).is_err());

        let unauthorized = br#"{"schema_version":"resume-ir.error.v1","request_id":"detail-request","status":"error","error":{"code":"UNAUTHORIZED","action":"authenticate"}}"#;
        let projected = project_error(unauthorized, 401, &expected).unwrap();
        assert!(serde_json::to_string(&projected)
            .unwrap()
            .contains("UNAUTHORIZED"));

        let missing_request = br#"{"schema_version":"resume-ir.error.v1","status":"error","error":{"code":"STALE_SELECTION","action":"refresh_search"}}"#;
        assert!(project_error(missing_request, 409, &expected).is_err());
    }

    #[test]
    fn search_errors_require_v1_unified_schema_and_exact_request_context() {
        let expected = ExpectedResponse::Search {
            request_id: "search-request".to_string(),
            max_results: 10,
        };
        let overloaded = br#"{"schema_version":"resume-ir.error.v1","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"message":"synthetic-private"},"private_debug":true}"#;
        let projected = project_error(overloaded, 503, &expected).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(exposed.contains("OVERLOADED"));
        assert!(exposed.contains("retry_after_ms"));
        assert!(!exposed.contains("synthetic-private"));
        assert!(!exposed.contains("private_debug"));

        let legacy = br#"{"schema_version":"resume-ir.ipc-response.v1","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","retry_after_ms":250}}"#;
        assert!(project_error(legacy, 503, &expected).is_err());
        let wrong_request = br#"{"schema_version":"resume-ir.error.v1","request_id":"other-request","status":"error","error":{"code":"SEMANTIC_DISABLED","action":"select_supported_mode"}}"#;
        assert!(project_error(wrong_request, 503, &expected).is_err());
    }
}
