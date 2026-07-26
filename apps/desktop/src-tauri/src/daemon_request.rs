use std::path::{Component, Path};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::daemon_client::DesktopError;
use crate::daemon_exchange::{
    valid_opaque_id, valid_stable_id, ExpectedResponse, PreparedDaemonRequest, SearchSelection,
};

const MAX_QUERY_BYTES: usize = 4096;
const MAX_FILTER_VALUE_BYTES: usize = 256;
const MAX_FILTER_VALUES: usize = 64;
const MAX_ROOT_PATH_BYTES: usize = 32 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_DETAIL_BODY_PAGE_BYTES: u64 = 32 * 1024;
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
const SOURCE_FILE_VERIFICATION_TIMEOUT: Duration = Duration::from_secs(15);
const SEARCH_RESPONSE_GRACE_MS: u64 = 1_000;
const SEARCH_RESPONSE_TIMEOUT_MAX_MS: u64 = 61_000;

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum DesktopRequest {
    Status,
    Diagnostics,
    Search(SearchRequest),
    Detail(DetailRequest),
    Hydrate(HydrateRequest),
    PreviewCreate(PreviewCreateRequest),
    PreviewRange(PreviewRangeRequest),
    PreviewClose(PreviewCloseRequest),
    Cancel(CancelRequest),
    RootControl(RootControlRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operation {
    Status,
    Diagnostics,
    Search,
    Detail,
    Hydrate,
    PreviewCreate,
    PreviewRange,
    PreviewClose,
    Cancel,
    RootControl,
    SourceRoots,
    RootDeletion,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootControlRequest {
    root_id: String,
    action: RootControlAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootControlAction {
    Pause,
    Resume,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchRequest {
    schema_version: String,
    request_id: String,
    client_capability: String,
    deadline_ms: u64,
    cancel_token: Option<String>,
    payload: SearchPayload,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchPayload {
    query: String,
    mode: SearchMode,
    top_k: usize,
    #[serde(default)]
    filters: SearchFilters,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchMode {
    Fulltext,
    Semantic,
    Hybrid,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    degree_min: Option<DegreeMinimum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skills_any: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    locations_any: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    years_experience_min: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum DegreeMinimum {
    Associate,
    Bachelor,
    Master,
    Doctorate,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetailRequest {
    schema_version: String,
    request_id: String,
    selection: SearchSelection,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HydrateRequest {
    schema_version: String,
    request_id: String,
    selection: SearchSelection,
    body_offset_bytes: u64,
    body_limit_bytes: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelRequest {
    schema_version: String,
    request_id: String,
    cancel_token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewCreateRequest {
    schema_version: String,
    request_id: String,
    selection: SearchSelection,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewRangeRequest {
    schema_version: String,
    request_id: String,
    lease_id: String,
    offset: u64,
    length: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewCloseRequest {
    schema_version: String,
    request_id: String,
    lease_id: String,
}

impl DesktopRequest {
    pub(crate) fn operation(&self) -> Operation {
        match self {
            Self::Status => Operation::Status,
            Self::Diagnostics => Operation::Diagnostics,
            Self::Search(_) => Operation::Search,
            Self::Detail(_) => Operation::Detail,
            Self::Hydrate(_) => Operation::Hydrate,
            Self::PreviewCreate(_) => Operation::PreviewCreate,
            Self::PreviewRange(_) => Operation::PreviewRange,
            Self::PreviewClose(_) => Operation::PreviewClose,
            Self::Cancel(_) => Operation::Cancel,
            Self::RootControl(_) => Operation::RootControl,
        }
    }

    pub(crate) fn prepare(self) -> Result<PreparedDaemonRequest, DesktopError> {
        match self {
            Self::Status => Ok(PreparedDaemonRequest::empty(
                ExpectedResponse::Status,
                DEFAULT_RESPONSE_TIMEOUT,
            )),
            Self::Diagnostics => Ok(PreparedDaemonRequest::empty(
                ExpectedResponse::Diagnostics,
                DEFAULT_RESPONSE_TIMEOUT,
            )),
            Self::Search(request) => {
                if request.schema_version != "resume-ir.ipc-request.v3"
                    || request.client_capability != "interactive_gui"
                    || !(1..=60_000).contains(&request.deadline_ms)
                    || !valid_opaque_id(&request.request_id)
                    || request
                        .cancel_token
                        .as_deref()
                        .is_some_and(|token| !valid_opaque_id(token))
                    || request.payload.query.trim().is_empty()
                    || request.payload.query.len() > MAX_QUERY_BYTES
                    || !(1..=100).contains(&request.payload.top_k)
                    || !valid_filters(&request.payload.filters)
                {
                    return Err(invalid_request());
                }
                let response_timeout = Duration::from_millis(
                    request
                        .deadline_ms
                        .saturating_add(SEARCH_RESPONSE_GRACE_MS)
                        .min(SEARCH_RESPONSE_TIMEOUT_MAX_MS),
                );
                let expected = ExpectedResponse::Search {
                    request_id: request.request_id.clone(),
                    max_results: request.payload.top_k,
                };
                PreparedDaemonRequest::new(serialize_body(&request)?, expected, response_timeout)
            }
            Self::Detail(request) => {
                if request.schema_version != "resume-ir.detail-request.v3"
                    || !valid_opaque_id(&request.request_id)
                    || !request.selection.is_valid()
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::Detail {
                    request_id: request.request_id.clone(),
                    selection: request.selection.clone(),
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    DEFAULT_RESPONSE_TIMEOUT,
                )
            }
            Self::Hydrate(request) => {
                if request.schema_version != "resume-ir.detail-hydrate-request.v3"
                    || !valid_opaque_id(&request.request_id)
                    || !request.selection.is_valid()
                    || request.body_offset_bytes > MAX_SAFE_INTEGER
                    || !(4..=MAX_DETAIL_BODY_PAGE_BYTES)
                        .contains(&u64::from(request.body_limit_bytes))
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::Hydrate {
                    request_id: request.request_id.clone(),
                    selection: request.selection.clone(),
                    body_offset_bytes: request.body_offset_bytes,
                    body_limit_bytes: request.body_limit_bytes,
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    DEFAULT_RESPONSE_TIMEOUT,
                )
            }
            Self::PreviewCreate(request) => {
                if request.schema_version != "resume-ir.source-preview-create-request.v1"
                    || !valid_opaque_id(&request.request_id)
                    || !request.selection.is_valid()
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::PreviewCreate {
                    request_id: request.request_id.clone(),
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    SOURCE_FILE_VERIFICATION_TIMEOUT,
                )
            }
            Self::PreviewRange(request) => {
                if request.schema_version != "resume-ir.source-preview-range-request.v1"
                    || !valid_opaque_id(&request.request_id)
                    || !valid_lease_id(&request.lease_id)
                    || request.offset > MAX_SAFE_INTEGER
                    || !(1..=64 * 1024).contains(&request.length)
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::PreviewRange {
                    request_id: request.request_id.clone(),
                    offset: request.offset,
                    max_bytes: request.length,
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    DEFAULT_RESPONSE_TIMEOUT,
                )
            }
            Self::PreviewClose(request) => {
                if request.schema_version != "resume-ir.source-preview-close-request.v1"
                    || !valid_opaque_id(&request.request_id)
                    || !valid_lease_id(&request.lease_id)
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::PreviewClose {
                    request_id: request.request_id.clone(),
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    DEFAULT_RESPONSE_TIMEOUT,
                )
            }
            Self::Cancel(request) => {
                if request.schema_version != "resume-ir.search-cancel-request.v1"
                    || !valid_opaque_id(&request.request_id)
                    || !valid_opaque_id(&request.cancel_token)
                {
                    return Err(invalid_request());
                }
                let expected = ExpectedResponse::Cancel {
                    request_id: request.request_id.clone(),
                };
                PreparedDaemonRequest::new(
                    serialize_body(&request)?,
                    expected,
                    DEFAULT_RESPONSE_TIMEOUT,
                )
            }
            Self::RootControl(_) => Err(invalid_request()),
        }
    }

    pub(crate) fn root_control(&self) -> Result<Option<(&str, RootControlAction)>, DesktopError> {
        let Self::RootControl(request) = self else {
            return Ok(None);
        };
        if !valid_stable_id(&request.root_id, "root-") {
            return Err(invalid_request());
        }
        Ok(Some((&request.root_id, request.action)))
    }
}

fn valid_lease_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Serialize)]
struct SourceRootRegisterRequest<'a> {
    schema_version: &'static str,
    requested_path: &'a str,
    display_label: &'a str,
}

#[derive(Serialize)]
struct LegacySourceRootMigrationRequest<'a> {
    schema_version: &'static str,
    roots: Vec<LegacySourceRootRegistration<'a>>,
}

#[derive(Serialize)]
struct LegacySourceRootRegistration<'a> {
    requested_path: &'a str,
    display_label: &'a str,
}

#[derive(Serialize)]
struct SourceRootRequest<'a> {
    schema_version: &'static str,
    root_id: &'a str,
}

#[derive(Serialize)]
struct SourceRootControlRequest<'a> {
    schema_version: &'static str,
    root_id: &'a str,
    action: RootControlAction,
}

pub(crate) fn prepare_source_root_register_request(
    root: &Path,
    display_label: &str,
) -> Result<PreparedDaemonRequest, DesktopError> {
    let root = validated_root_path(root)?;
    if display_label.is_empty()
        || display_label.chars().count() > 80
        || display_label.chars().any(char::is_control)
    {
        return Err(invalid_request());
    }
    PreparedDaemonRequest::new(
        serialize_body(&SourceRootRegisterRequest {
            schema_version: "resume-ir.source-root-register-request.v1",
            requested_path: root,
            display_label,
        })?,
        ExpectedResponse::SourceRoots,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

pub(crate) fn prepare_legacy_source_root_migration_request(
    roots: &[(&Path, &str)],
) -> Result<PreparedDaemonRequest, DesktopError> {
    if roots.is_empty() || roots.len() > 16 {
        return Err(invalid_request());
    }
    let mut registrations = Vec::with_capacity(roots.len());
    for (root, display_label) in roots {
        let requested_path = validated_root_path(root)?;
        if display_label.is_empty()
            || display_label.chars().count() > 80
            || display_label.chars().any(char::is_control)
        {
            return Err(invalid_request());
        }
        registrations.push(LegacySourceRootRegistration {
            requested_path,
            display_label,
        });
    }
    PreparedDaemonRequest::new(
        serialize_body(&LegacySourceRootMigrationRequest {
            schema_version: "resume-ir.source-root-legacy-migration-request.v1",
            roots: registrations,
        })?,
        ExpectedResponse::SourceRoots,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

pub(crate) fn prepare_source_root_scan_request(
    root_id: &str,
) -> Result<PreparedDaemonRequest, DesktopError> {
    if !valid_stable_id(root_id, "root-") {
        return Err(invalid_request());
    }
    PreparedDaemonRequest::new(
        serialize_body(&SourceRootRequest {
            schema_version: "resume-ir.source-root-scan-request.v1",
            root_id,
        })?,
        ExpectedResponse::SourceRoots,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

pub(crate) fn prepare_source_root_control_request(
    root_id: &str,
    action: RootControlAction,
) -> Result<PreparedDaemonRequest, DesktopError> {
    if !valid_stable_id(root_id, "root-") {
        return Err(invalid_request());
    }
    PreparedDaemonRequest::new(
        serialize_body(&SourceRootControlRequest {
            schema_version: "resume-ir.source-root-control-request.v1",
            root_id,
            action,
        })?,
        ExpectedResponse::SourceRoots,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

pub(crate) fn prepare_source_root_delete_request(
    root_id: &str,
) -> Result<PreparedDaemonRequest, DesktopError> {
    if !valid_stable_id(root_id, "root-") {
        return Err(invalid_request());
    }
    PreparedDaemonRequest::new(
        serialize_body(&SourceRootRequest {
            schema_version: "resume-ir.source-root-delete-request.v1",
            root_id,
        })?,
        ExpectedResponse::RootDeletion,
        DEFAULT_RESPONSE_TIMEOUT,
    )
}

fn validated_root_path(root: &Path) -> Result<&str, DesktopError> {
    let root = root.to_str().ok_or_else(invalid_request)?;
    let path = Path::new(root);
    if !path.is_absolute()
        || root.is_empty()
        || root.len() > MAX_ROOT_PATH_BYTES
        || root.contains('\0')
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid_request());
    }
    Ok(root)
}

fn serialize_body<T: Serialize>(body: &T) -> Result<Vec<u8>, DesktopError> {
    serde_json::to_vec(body).map_err(|_| invalid_request())
}

fn valid_filters(filters: &SearchFilters) -> bool {
    valid_filter_values(&filters.skills_any)
        && valid_filter_values(&filters.locations_any)
        && filters
            .years_experience_min
            .is_none_or(|years| years.is_finite() && (0.0..=100.0).contains(&years))
}

fn valid_filter_values(values: &[String]) -> bool {
    values.len() <= MAX_FILTER_VALUES
        && values
            .iter()
            .all(|value| !value.trim().is_empty() && value.len() <= MAX_FILTER_VALUE_BYTES)
}

fn invalid_request() -> DesktopError {
    DesktopError::new("request_invalid", "桌面请求合同无效")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_requests_reject_unbounded_or_mismatched_payloads() {
        let valid_search: DesktopRequest = serde_json::from_str(r#"{"operation":"search","body":{"schema_version":"resume-ir.ipc-request.v3","request_id":"gui-search-1","client_capability":"interactive_gui","deadline_ms":5000,"cancel_token":"gui-cancel-1","payload":{"query":"Java Kafka","mode":"hybrid","top_k":50,"filters":{"degree_min":"master","skills_any":["Java","Kafka"],"locations_any":["Shanghai"],"years_experience_min":5.0}}}}"#).unwrap();
        let prepared = valid_search.prepare().unwrap();
        assert_eq!(prepared.response_timeout(), Duration::from_secs(6));
        assert!(matches!(
            prepared.expected(),
            ExpectedResponse::Search {
                request_id,
                max_results: 50
            } if request_id == "gui-search-1"
        ));
        let status: DesktopRequest = serde_json::from_str(r#"{"operation":"status"}"#).unwrap();
        let status = status.prepare().unwrap();
        assert!(status.body().is_empty());
        assert!(matches!(status.expected(), ExpectedResponse::Status));
        for invalid in [
            r#"{"operation":"status","body":{}}"#,
            r#"{"operation":"search","body":{"schema_version":"resume-ir.ipc-request.v2","request_id":"gui-search-1","client_capability":"interactive_gui","deadline_ms":1500,"payload":{"query":"x","mode":"fulltext","top_k":101,"filters":{}}}}"#,
            r#"{"operation":"detail","body":{"doc_id":"preview-id"}}"#,
            r#"{"operation":"detail","body":{"schema_version":"resume-ir.detail-request.v2","request_id":"detail-1","selection":{"doc_id":"doc_00000000000000000000000000000000","version_id":"ver_00000000000000000000000000000000","visible_epoch":7}}}"#,
            r#"{"operation":"hydrate","body":{"schema_version":"resume-ir.detail-hydrate-request.v1","request_id":"hydrate-1","doc_id":"doc_00000000000000000000000000000000","body_offset_bytes":0,"body_limit_bytes":32768}}"#,
        ] {
            let request = serde_json::from_str::<DesktopRequest>(invalid);
            assert!(request.is_err() || request.unwrap().prepare().is_err());
        }
    }

    #[test]
    fn detail_and_hydrate_prepare_exact_v3_response_context() {
        let detail: DesktopRequest = serde_json::from_str(r#"{"operation":"detail","body":{"schema_version":"resume-ir.detail-request.v3","request_id":"detail-1","selection":{"doc_id":"doc_00000000000000000000000000000000","version_id":"ver_00000000000000000000000000000000","visible_epoch":7}}}"#).unwrap();
        let prepared = detail.prepare().unwrap();
        assert!(matches!(
            prepared.expected(),
            ExpectedResponse::Detail {
                request_id,
                selection
            } if request_id == "detail-1" && selection.visible_epoch() == 7
        ));

        let hydrate: DesktopRequest = serde_json::from_str(r#"{"operation":"hydrate","body":{"schema_version":"resume-ir.detail-hydrate-request.v3","request_id":"hydrate-1","selection":{"doc_id":"doc_00000000000000000000000000000000","version_id":"ver_00000000000000000000000000000000","visible_epoch":7},"body_offset_bytes":32768,"body_limit_bytes":32768}}"#).unwrap();
        let prepared = hydrate.prepare().unwrap();
        assert!(matches!(
            prepared.expected(),
            ExpectedResponse::Hydrate {
                request_id,
                selection,
                body_offset_bytes: 32768,
                body_limit_bytes: 32768,
            } if request_id == "hydrate-1" && selection.visible_epoch() == 7
        ));
    }

    #[test]
    fn root_control_accepts_only_one_authorized_handle_shape_and_closed_action() {
        let valid: DesktopRequest = serde_json::from_str(
            r#"{"operation":"root_control","body":{"root_id":"root-00000000000000000000000000000000","action":"pause"}}"#,
        )
        .unwrap();
        assert!(matches!(valid.operation(), Operation::RootControl));
        let (handle, action) = valid.root_control().unwrap().unwrap();
        assert_eq!(handle, "root-00000000000000000000000000000000");
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            serde_json::json!("pause")
        );

        for invalid in [
            r#"{"operation":"root_control","body":{"root_id":"/private/synthetic","action":"pause"}}"#,
            r#"{"operation":"root_control","body":{"root_id":"root-00000000000000000000000000000000","action":"remove"}}"#,
            r#"{"operation":"root_control","body":{"root_id":"root-00000000000000000000000000000000","action":"pause","extra":true}}"#,
        ] {
            let request = serde_json::from_str::<DesktopRequest>(invalid);
            assert!(request.is_err() || request.unwrap().root_control().is_err());
        }
    }
}
