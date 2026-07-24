use serde::{Deserialize, Serialize};

use super::enums::ErrorStatus;
use super::{decode, ensure, ensure_schema};
use crate::daemon_client::DesktopError;
use crate::daemon_exchange::{valid_opaque_id, ExpectedResponse};

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ErrorBody {
    Unified(UnifiedErrorBody),
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    schema_version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnifiedErrorBody {
    schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    status: ErrorStatus,
    error: UnifiedError,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UnifiedError {
    code: UnifiedErrorCode,
    action: UnifiedErrorAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
    #[serde(deserialize_with = "required_nullable")]
    capability: Option<CapabilityName>,
    #[serde(deserialize_with = "required_nullable")]
    reason: Option<FailureReason>,
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
    ServiceInitializing,
    ServiceBlocked,
    CapabilityUnavailable,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UnifiedErrorAction {
    Authenticate,
    CorrectRequest,
    RefreshSearch,
    ReducePageSize,
    WaitForRepair,
    RepairRequired,
    Retry,
    SelectSupportedMode,
    WaitForService,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CapabilityName {
    KeywordSearch,
    Detail,
    SemanticSearch,
    HybridSearch,
    TextImport,
    OcrImport,
    IndexPublication,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FailureReason {
    EmbeddingUnavailable,
    OcrUnavailable,
    ClassifierUnavailable,
    MetadataInitializing,
    MigrationRebuild,
    ArtifactUnavailable,
    SourceUnavailable,
    RuntimeInvariant,
    UnsupportedStoreSchema,
    MetadataUnavailable,
}

pub(super) fn project_error(
    body: &[u8],
    http_status: u16,
    expected: &ExpectedResponse,
) -> Result<ErrorBody, DesktopError> {
    let envelope: ErrorEnvelope = decode(body)?;
    ensure_schema(&envelope.schema_version, "resume-ir.error.v2")?;
    project_unified_error(body, http_status, expected).map(ErrorBody::Unified)
}

fn project_unified_error(
    body: &[u8],
    http_status: u16,
    expected: &ExpectedResponse,
) -> Result<UnifiedErrorBody, DesktopError> {
    let value: UnifiedErrorBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.error.v2")?;
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
    ensure(context_matches(&value.error, expected))?;
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
        UnifiedErrorCode::ServiceInitializing
        | UnifiedErrorCode::ServiceBlocked
        | UnifiedErrorCode::CapabilityUnavailable => 503,
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
            UnifiedErrorCode::QueryServiceUnavailable,
            UnifiedErrorAction::RepairRequired
        ) | (
            UnifiedErrorCode::ServiceInitializing,
            UnifiedErrorAction::WaitForService
        ) | (
            UnifiedErrorCode::ServiceBlocked,
            UnifiedErrorAction::RepairRequired | UnifiedErrorAction::Retry
        ) | (
            UnifiedErrorCode::CapabilityUnavailable,
            UnifiedErrorAction::SelectSupportedMode
        ) | (
            UnifiedErrorCode::Conflict
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::Overloaded
                | UnifiedErrorCode::Internal,
            UnifiedErrorAction::Retry
        )
    )
}

fn code_allowed_for_response(code: &UnifiedErrorCode, expected: &ExpectedResponse) -> bool {
    match expected {
        ExpectedResponse::Status | ExpectedResponse::Diagnostics => false,
        ExpectedResponse::Search { .. } => matches!(
            code,
            UnifiedErrorCode::Unauthorized
                | UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::Conflict
                | UnifiedErrorCode::NotFound
                | UnifiedErrorCode::LimitExceeded
                | UnifiedErrorCode::Repairing
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
                | UnifiedErrorCode::SemanticDisabled
                | UnifiedErrorCode::Overloaded
                | UnifiedErrorCode::Internal
                | UnifiedErrorCode::ServiceInitializing
                | UnifiedErrorCode::ServiceBlocked
                | UnifiedErrorCode::CapabilityUnavailable
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
                | UnifiedErrorCode::ServiceInitializing
                | UnifiedErrorCode::ServiceBlocked
        ),
        ExpectedResponse::Import | ExpectedResponse::RootControl => matches!(
            code,
            UnifiedErrorCode::Unauthorized
                | UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::Conflict
                | UnifiedErrorCode::NotFound
                | UnifiedErrorCode::LimitExceeded
                | UnifiedErrorCode::Repairing
                | UnifiedErrorCode::MetadataUnavailable
                | UnifiedErrorCode::QueryServiceUnavailable
                | UnifiedErrorCode::ServiceInitializing
                | UnifiedErrorCode::ServiceBlocked
                | UnifiedErrorCode::CapabilityUnavailable
        ),
        ExpectedResponse::Cancel { .. } => matches!(
            code,
            UnifiedErrorCode::Unauthorized
                | UnifiedErrorCode::BadRequest
                | UnifiedErrorCode::ServiceInitializing
                | UnifiedErrorCode::ServiceBlocked
        ),
    }
}

fn context_matches(error: &UnifiedError, expected: &ExpectedResponse) -> bool {
    match error.code {
        UnifiedErrorCode::ServiceInitializing => {
            error.capability.is_none()
                && matches!(
                    error.reason,
                    Some(
                        FailureReason::MetadataInitializing
                            | FailureReason::MigrationRebuild
                            | FailureReason::ArtifactUnavailable
                    )
                )
        }
        UnifiedErrorCode::ServiceBlocked => {
            error.capability.is_none()
                && matches!(
                    error.reason,
                    Some(
                        FailureReason::SourceUnavailable
                            | FailureReason::RuntimeInvariant
                            | FailureReason::UnsupportedStoreSchema
                            | FailureReason::MetadataUnavailable
                    )
                )
        }
        UnifiedErrorCode::CapabilityUnavailable => {
            error.capability.is_some()
                && capability_reason_matches(expected, error.capability, error.reason)
        }
        _ => error.capability.is_none() && error.reason.is_none(),
    }
}

fn capability_reason_matches(
    expected: &ExpectedResponse,
    capability: Option<CapabilityName>,
    reason: Option<FailureReason>,
) -> bool {
    match expected {
        ExpectedResponse::Search { .. } => matches!(
            (capability, reason),
            (
                Some(CapabilityName::SemanticSearch),
                Some(FailureReason::EmbeddingUnavailable),
            )
        ),
        ExpectedResponse::Import | ExpectedResponse::RootControl => matches!(
            (capability, reason),
            (
                Some(CapabilityName::TextImport),
                Some(FailureReason::EmbeddingUnavailable | FailureReason::ClassifierUnavailable),
            )
        ),
        ExpectedResponse::Status
        | ExpectedResponse::Diagnostics
        | ExpectedResponse::Detail { .. }
        | ExpectedResponse::Hydrate { .. }
        | ExpectedResponse::Cancel { .. } => false,
    }
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
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
        let stale = br#"{"schema_version":"resume-ir.error.v2","request_id":"detail-request","status":"error","error":{"code":"STALE_SELECTION","action":"refresh_search","capability":null,"reason":null}}"#;
        let projected = project_error(stale, 409, &expected).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(exposed.contains("STALE_SELECTION"));

        let wrong_action = br#"{"schema_version":"resume-ir.error.v2","request_id":"detail-request","status":"error","error":{"code":"STALE_SELECTION","action":"retry","capability":null,"reason":null}}"#;
        assert!(project_error(wrong_action, 409, &expected).is_err());
        assert!(project_error(stale, 404, &expected).is_err());
        assert!(project_error(stale, 409, &ExpectedResponse::Status).is_err());

        let unauthorized = br#"{"schema_version":"resume-ir.error.v2","request_id":"detail-request","status":"error","error":{"code":"UNAUTHORIZED","action":"authenticate","capability":null,"reason":null}}"#;
        let projected = project_error(unauthorized, 401, &expected).unwrap();
        assert!(serde_json::to_string(&projected)
            .unwrap()
            .contains("UNAUTHORIZED"));

        let missing_request = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"STALE_SELECTION","action":"refresh_search","capability":null,"reason":null}}"#;
        assert!(project_error(missing_request, 409, &expected).is_err());
    }

    #[test]
    fn search_errors_require_v2_schema_exact_shape_and_context() {
        let expected = ExpectedResponse::Search {
            request_id: "search-request".to_string(),
            max_results: 10,
        };
        let overloaded = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"capability":null,"reason":null}}"#;
        let projected = project_error(overloaded, 503, &expected).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(exposed.contains("OVERLOADED"));
        assert!(exposed.contains("retry_after_ms"));
        let legacy = br#"{"schema_version":"resume-ir.error.v1","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"capability":null,"reason":null}}"#;
        assert!(project_error(legacy, 503, &expected).is_err());
        let wrong_request = br#"{"schema_version":"resume-ir.error.v2","request_id":"other-request","status":"error","error":{"code":"SEMANTIC_DISABLED","action":"select_supported_mode","capability":null,"reason":null}}"#;
        assert!(project_error(wrong_request, 503, &expected).is_err());

        let missing_required_null = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"reason":null}}"#;
        assert!(project_error(missing_required_null, 503, &expected).is_err());
        let unknown = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"capability":null,"reason":null,"private_debug":true}}"#;
        assert!(project_error(unknown, 503, &expected).is_err());

        let wrong_query_service_action = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"QUERY_SERVICE_UNAVAILABLE","action":"retry","capability":null,"reason":null}}"#;
        assert!(project_error(wrong_query_service_action, 503, &expected).is_err());
    }

    #[test]
    fn capability_error_reason_is_operation_specific() {
        let expected = ExpectedResponse::Search {
            request_id: "search-request".to_string(),
            max_results: 10,
        };
        let semantic = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"semantic_search","reason":"embedding_unavailable"}}"#;
        assert!(project_error(semantic, 503, &expected).is_ok());
        let impossible = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"semantic_search","reason":"ocr_unavailable"}}"#;
        assert!(project_error(impossible, 503, &expected).is_err());

        let unrelated = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"ocr_import","reason":"ocr_unavailable"}}"#;
        assert!(project_error(unrelated, 503, &expected).is_err());
        let hybrid = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"hybrid_search","reason":"embedding_unavailable"}}"#;
        assert!(project_error(hybrid, 503, &expected).is_err());
        let core_alias = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"semantic_search","reason":"core_initializing"}}"#;
        assert!(project_error(core_alias, 503, &expected).is_err());

        let import_embedding = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"text_import","reason":"embedding_unavailable"}}"#;
        assert!(project_error(import_embedding, 503, &ExpectedResponse::Import).is_ok());
        let import_classifier = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"text_import","reason":"classifier_unavailable"}}"#;
        assert!(project_error(import_classifier, 503, &ExpectedResponse::RootControl).is_ok());
        let import_semantic = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"CAPABILITY_UNAVAILABLE","action":"select_supported_mode","capability":"semantic_search","reason":"embedding_unavailable"}}"#;
        assert!(project_error(import_semantic, 503, &ExpectedResponse::Import).is_err());
    }

    #[test]
    fn control_plane_routes_accept_only_their_http_200_success_contracts() {
        let query_unavailable = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"QUERY_SERVICE_UNAVAILABLE","action":"repair_required","capability":null,"reason":null}}"#;
        assert!(project_error(query_unavailable, 503, &ExpectedResponse::Status).is_err());
        assert!(project_error(query_unavailable, 503, &ExpectedResponse::Diagnostics).is_err());

        let unauthorized = br#"{"schema_version":"resume-ir.error.v2","status":"error","error":{"code":"UNAUTHORIZED","action":"authenticate","capability":null,"reason":null}}"#;
        assert!(project_error(unauthorized, 401, &ExpectedResponse::Status).is_err());
        assert!(project_error(unauthorized, 401, &ExpectedResponse::Diagnostics).is_err());

        let initializing = br#"{"schema_version":"resume-ir.error.v2","request_id":"search-request","status":"error","error":{"code":"SERVICE_INITIALIZING","action":"wait_for_service","capability":null,"reason":"metadata_initializing"}}"#;
        let search = ExpectedResponse::Search {
            request_id: "search-request".to_string(),
            max_results: 10,
        };
        assert!(project_error(initializing, 503, &search).is_ok());
    }
}
