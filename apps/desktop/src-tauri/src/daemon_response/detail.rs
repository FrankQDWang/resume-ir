use serde::{Deserialize, Serialize};

use super::enums::{AcceptedStatus, CancelStatus, DetailFieldType, ImportProfile, OkStatus};
use super::{
    bounded_chars, decode, ensure, ensure_schema, protocol_error, DesktopError, SafeCount,
};
use crate::daemon_exchange::{valid_opaque_id, valid_stable_id, ExpectedResponse, SearchSelection};

const MAX_DETAIL_FIELDS: usize = 256;
const MAX_UI_DETAIL_FIELDS: usize = 32;
const MAX_BODY_PAGE_BYTES: usize = 32 * 1024;
const MAX_DETAIL_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_VERSION_LABEL_BYTES: usize = 256;
const MAX_LANGUAGE_COUNT: usize = 64;
const MAX_LANGUAGE_BYTES: usize = 64;

#[derive(Deserialize, Serialize)]
pub(super) struct DetailBody {
    schema_version: String,
    request_id: String,
    selection: SearchSelection,
    status: OkStatus,
    document: DetailDocument,
    limits: DetailLimits,
}

#[derive(Deserialize, Serialize)]
struct DetailDocument {
    source_byte_size: SafeCount,
    parse_version: String,
    schema_version: String,
    language_set: Vec<String>,
    page_count: Option<u32>,
    quality_score: Option<f64>,
    #[serde(skip_serializing)]
    field_limit: usize,
    #[serde(skip_serializing)]
    field_count_total: usize,
    #[serde(skip_serializing)]
    field_count_returned: usize,
    fields_truncated: bool,
    fields: Vec<DetailField>,
    snippet: String,
}

#[derive(Deserialize, Serialize)]
struct DetailLimits {
    max_fields: usize,
    max_response_bytes: usize,
}

#[derive(Deserialize, Serialize)]
struct DetailField {
    #[serde(rename = "type")]
    field_type: DetailFieldType,
    value: String,
    confidence: f64,
}

#[derive(Deserialize, Serialize)]
pub(super) struct HydrateBody {
    schema_version: String,
    request_id: String,
    selection: SearchSelection,
    status: OkStatus,
    document: HydrateDocument,
    privacy: HydratePrivacy,
    limits: HydrateLimits,
}

#[derive(Deserialize, Serialize)]
struct HydrateDocument {
    body_page: HydratePage,
}

#[derive(Deserialize, Serialize)]
struct HydratePage {
    encoding: Utf8Encoding,
    offset_bytes: u64,
    next_offset_bytes: u64,
    total_bytes: u64,
    complete: bool,
    text: String,
}

#[derive(Deserialize, Serialize)]
enum Utf8Encoding {
    #[serde(rename = "utf-8")]
    Utf8,
}

#[derive(Deserialize, Serialize)]
struct HydratePrivacy {
    local_authenticated_only: bool,
    public_output_allowed: bool,
}

#[derive(Deserialize, Serialize)]
struct HydrateLimits {
    max_body_page_bytes: u64,
    max_response_bytes: u64,
}

#[derive(Deserialize, Serialize)]
pub(super) struct ImportBody {
    schema_version: String,
    status: AcceptedStatus,
    accepted_roots: usize,
    new_tasks: usize,
    #[serde(skip_serializing)]
    task_ids: Vec<String>,
    scan_profile: ImportProfile,
    scan_file_limit: Option<u64>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct CancelBody {
    schema_version: String,
    request_id: String,
    status: CancelStatus,
}

pub(super) fn project_detail(
    body: &[u8],
    expected: &ExpectedResponse,
) -> Result<DetailBody, DesktopError> {
    let ExpectedResponse::Detail {
        request_id,
        selection,
    } = expected
    else {
        return Err(protocol_error());
    };
    let mut value: DetailBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.detail-response.v3")?;
    let document = &mut value.document;
    ensure(
        body.len() <= MAX_DETAIL_RESPONSE_BYTES
            && value.request_id == *request_id
            && valid_opaque_id(&value.request_id)
            && value.selection == *selection
            && value.selection.is_valid()
            && !document.parse_version.is_empty()
            && document.parse_version.len() <= MAX_VERSION_LABEL_BYTES
            && !document.schema_version.is_empty()
            && document.schema_version.len() <= MAX_VERSION_LABEL_BYTES
            && document.language_set.len() <= MAX_LANGUAGE_COUNT
            && document.language_set.iter().all(|language| {
                !language.trim().is_empty() && language.len() <= MAX_LANGUAGE_BYTES
            })
            && document
                .quality_score
                .is_none_or(|score| score.is_finite() && (0.0..=1.0).contains(&score))
            && document.field_limit == MAX_DETAIL_FIELDS
            && document.fields.len() <= MAX_DETAIL_FIELDS
            && document.field_count_returned == document.fields.len()
            && (document.field_count_returned..=MAX_DETAIL_FIELDS)
                .contains(&document.field_count_total)
            && document.fields_truncated
                == (document.field_count_returned < document.field_count_total)
            && bounded_chars(&document.snippet, 243, 4096)
            && value.limits.max_fields == MAX_DETAIL_FIELDS
            && value.limits.max_response_bytes == MAX_DETAIL_RESPONSE_BYTES,
    )?;
    for field in &document.fields {
        ensure(
            !field.value.trim().is_empty()
                && bounded_chars(&field.value, 123, 1024)
                && field.confidence.is_finite()
                && (0.0..=1.0).contains(&field.confidence),
        )?;
    }
    let projected_truncated =
        document.fields_truncated || document.fields.len() > MAX_UI_DETAIL_FIELDS;
    document.fields_truncated = projected_truncated;
    document.fields.truncate(MAX_UI_DETAIL_FIELDS);
    Ok(value)
}

pub(super) fn project_hydrate(
    body: &[u8],
    body_bytes: usize,
    expected: &ExpectedResponse,
) -> Result<HydrateBody, DesktopError> {
    let ExpectedResponse::Hydrate {
        request_id,
        selection,
        body_offset_bytes,
        body_limit_bytes,
    } = expected
    else {
        return Err(protocol_error());
    };
    let value: HydrateBody = decode(body)?;
    ensure_schema(
        &value.schema_version,
        "resume-ir.detail-hydrate-response.v3",
    )?;
    let page = &value.document.body_page;
    ensure(
        body_bytes <= MAX_DETAIL_RESPONSE_BYTES
            && value.request_id == *request_id
            && valid_opaque_id(&value.request_id)
            && value.selection == *selection
            && value.selection.is_valid()
            && page.offset_bytes == *body_offset_bytes
            && page.text.len() <= MAX_BODY_PAGE_BYTES.min(*body_limit_bytes as usize)
            && page.offset_bytes <= page.next_offset_bytes
            && page.next_offset_bytes <= page.total_bytes
            && page.total_bytes <= super::MAX_SAFE_INTEGER
            && page.next_offset_bytes - page.offset_bytes == page.text.len() as u64
            && page.complete == (page.next_offset_bytes == page.total_bytes)
            && (page.complete || page.next_offset_bytes > page.offset_bytes)
            && value.privacy.local_authenticated_only
            && !value.privacy.public_output_allowed
            && value.limits.max_body_page_bytes == MAX_BODY_PAGE_BYTES as u64
            && value.limits.max_response_bytes == MAX_DETAIL_RESPONSE_BYTES as u64,
    )?;
    Ok(value)
}

pub(super) fn project_import(body: &[u8]) -> Result<ImportBody, DesktopError> {
    let value: ImportBody = decode(body)?;
    ensure_schema(&value.schema_version, "daemon.import.v1")?;
    ensure(
        value.accepted_roots == 1
            && value.new_tasks <= 1
            && value.task_ids.len() == 1
            && value
                .task_ids
                .iter()
                .all(|task_id| valid_stable_id(task_id, "imp_"))
            && value.scan_file_limit.is_none(),
    )?;
    Ok(value)
}

pub(super) fn project_cancel(
    body: &[u8],
    expected: &ExpectedResponse,
) -> Result<CancelBody, DesktopError> {
    let ExpectedResponse::Cancel { request_id } = expected else {
        return Err(protocol_error());
    };
    let value: CancelBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.search-cancel-response.v1")?;
    ensure(valid_opaque_id(&value.request_id) && value.request_id == *request_id)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> SearchSelection {
        serde_json::from_value(serde_json::json!({
            "doc_id": "doc_00000000000000000000000000000000",
            "version_id": "ver_00000000000000000000000000000000",
            "visible_epoch": 7,
        }))
        .unwrap()
    }

    fn expected_detail() -> ExpectedResponse {
        ExpectedResponse::Detail {
            request_id: "detail-request".to_string(),
            selection: selection(),
        }
    }

    fn expected_hydrate() -> ExpectedResponse {
        ExpectedResponse::Hydrate {
            request_id: "hydrate-request".to_string(),
            selection: selection(),
            body_offset_bytes: 0,
            body_limit_bytes: 32 * 1024,
        }
    }

    #[test]
    fn v3_detail_strictly_echoes_selection_and_drops_field_evidence() {
        let mut payload = serde_json::json!({
            "schema_version": "resume-ir.detail-response.v3",
            "request_id": "detail-request",
            "selection": selection(),
            "status": "ok",
            "document": {
                "source_byte_size": 1024,
                "parse_version": "parser-v1",
                "schema_version": "schema-v27",
                "language_set": ["en"],
                "page_count": 1,
                "quality_score": 0.9,
                "field_limit": 256,
                "field_count_total": 33,
                "field_count_returned": 33,
                "fields_truncated": false,
                "fields": [],
                "snippet": "synthetic snippet",
                "display_path": "synthetic-private-path",
                "visibility": "searchable",
                "document_status": "searchable",
                "private_debug": "synthetic-private-value"
            },
            "limits": {"max_fields": 256, "max_response_bytes": 1048576}
        });
        let field = serde_json::json!({"type":"skill","value":"synthetic-skill","confidence":0.9,"evidence":"synthetic-private-evidence","extractor":"synthetic-private-extractor"});
        payload["document"]["fields"] = serde_json::Value::Array(vec![field; 33]);
        let projected =
            project_detail(&serde_json::to_vec(&payload).unwrap(), &expected_detail()).unwrap();
        let exposed = serde_json::to_value(projected).unwrap();
        assert_eq!(exposed["document"]["fields"].as_array().unwrap().len(), 32);
        assert_eq!(exposed["document"]["fields_truncated"], true);
        let exposed = exposed.to_string();
        assert!(!exposed.contains("evidence"));
        assert!(!exposed.contains("extractor"));
        assert!(!exposed.contains("private_debug"));
        assert!(!exposed.contains("display_path"));
        assert!(!exposed.contains("synthetic-private-path"));
        assert!(!exposed.contains("visibility"));
        assert!(!exposed.contains("document_status"));

        payload["request_id"] = serde_json::json!("other-request");
        assert!(
            project_detail(&serde_json::to_vec(&payload).unwrap(), &expected_detail()).is_err()
        );
        payload["request_id"] = serde_json::json!("detail-request");
        payload["selection"]["visible_epoch"] = serde_json::json!(8);
        assert!(
            project_detail(&serde_json::to_vec(&payload).unwrap(), &expected_detail()).is_err()
        );
        payload["selection"]["visible_epoch"] = serde_json::json!(7);
        payload["schema_version"] = serde_json::json!("daemon.detail.v2");
        assert!(
            project_detail(&serde_json::to_vec(&payload).unwrap(), &expected_detail()).is_err()
        );
    }

    #[test]
    fn v3_hydrate_requires_exact_context_privacy_and_page_cursor() {
        let mut payload = serde_json::json!({
            "schema_version": "resume-ir.detail-hydrate-response.v3",
            "request_id": "hydrate-request",
            "selection": selection(),
            "status": "ok",
            "document": {
                "display_path": "synthetic-private-path",
                "body_page": {
                    "encoding": "utf-8",
                    "offset_bytes": 0,
                    "next_offset_bytes": 9,
                    "total_bytes": 9,
                    "complete": true,
                    "text": "synthetic"
                }
            },
            "privacy": {"local_authenticated_only": true, "public_output_allowed": false},
            "limits": {"max_body_page_bytes": 32768, "max_response_bytes": 1048576}
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let projected = project_hydrate(&bytes, bytes.len(), &expected_hydrate()).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(!exposed.contains("display_path"));
        assert!(!exposed.contains("synthetic-private-path"));

        payload["privacy"]["public_output_allowed"] = serde_json::Value::Bool(true);
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(project_hydrate(&bytes, bytes.len(), &expected_hydrate()).is_err());

        payload["privacy"]["public_output_allowed"] = serde_json::Value::Bool(false);
        payload["document"]["body_page"]["offset_bytes"] = serde_json::json!(4);
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(project_hydrate(&bytes, bytes.len(), &expected_hydrate()).is_err());

        payload["document"]["body_page"]["offset_bytes"] = serde_json::json!(0);
        payload["schema_version"] = serde_json::json!("resume-ir.detail-hydrate-response.v1");
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(project_hydrate(&bytes, bytes.len(), &expected_hydrate()).is_err());
    }
}
