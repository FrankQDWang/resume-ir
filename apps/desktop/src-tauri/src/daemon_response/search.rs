use serde::{Deserialize, Serialize};

use super::enums::{PartialReason, QueryMode, SearchStatus};
use super::{decode, ensure, ensure_schema, SafeCount};
use crate::daemon_client::DesktopError;
use crate::daemon_exchange::{valid_opaque_id, ExpectedResponse, SearchSelection};

const MAX_RESULTS: usize = 100;
const MAX_FILE_NAME_BYTES: usize = 160;
const MAX_SNIPPET_BYTES: usize = 4096;

#[derive(Deserialize, Serialize)]
pub(super) struct SearchBody {
    schema_version: String,
    request_id: String,
    status: SearchStatus,
    visible_epoch: SafeCount,
    query_mode: QueryMode,
    partial: bool,
    partial_reasons: Vec<PartialReason>,
    latency_ms: f64,
    result_count: usize,
    results: Vec<SearchHit>,
}

#[derive(Deserialize, Serialize)]
struct SearchHit {
    rank: usize,
    selection: SearchSelection,
    file_name: String,
    snippet: String,
}

pub(super) fn project_search(
    body: &[u8],
    expected: &ExpectedResponse,
) -> Result<SearchBody, DesktopError> {
    let ExpectedResponse::Search {
        request_id,
        max_results,
    } = expected
    else {
        return Err(super::protocol_error());
    };
    let value: SearchBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.search-response.v3")?;
    ensure(
        value.request_id == *request_id
            && valid_opaque_id(&value.request_id)
            && value.latency_ms.is_finite()
            && (0.0..=120_000.0).contains(&value.latency_ms)
            && value.results.len() <= MAX_RESULTS.min(*max_results)
            && value.result_count == value.results.len()
            && value.partial != value.partial_reasons.is_empty()
            && value.partial_reasons.len() <= 2
            && (value.status != SearchStatus::Cancelled
                || (value.results.is_empty() && !value.partial)),
    )?;
    for (index, hit) in value.results.iter().enumerate() {
        ensure(
            hit.rank == index + 1
                && hit.selection.is_valid()
                && hit.selection.visible_epoch() == value.visible_epoch.value()
                && !hit.file_name.is_empty()
                && hit.file_name.len() <= MAX_FILE_NAME_BYTES
                && hit.snippet.len() <= MAX_SNIPPET_BYTES,
        )?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> ExpectedResponse {
        ExpectedResponse::Search {
            request_id: "synthetic-search-request".to_string(),
            max_results: 10,
        }
    }

    #[test]
    fn v3_search_requires_nested_selection_and_exact_epoch() {
        let payload = br#"{"schema_version":"resume-ir.search-response.v3","request_id":"synthetic-search-request","status":"ok","visible_epoch":4,"query_mode":"hybrid","partial":false,"partial_reasons":[],"latency_ms":4.0,"stage_latency_ms":{"ann":1.0},"search_index":"available","result_count":1,"results":[{"rank":1,"selection":{"doc_id":"doc_00000000000000000000000000000000","version_id":"ver_00000000000000000000000000000000","visible_epoch":4},"file_name":"synthetic.pdf","snippet":"synthetic result","soft_dedupe":{"private_hint":"synthetic-private-value"}}]}"#;
        let projected = project_search(payload, &expected()).unwrap();
        let exposed = serde_json::to_string(&projected).unwrap();
        assert!(exposed.contains("\"selection\""));
        assert!(!exposed.contains("soft_dedupe"));
        assert!(!exposed.contains("synthetic-private-value"));
        assert!(!exposed.contains("stage_latency_ms"));
        assert!(!exposed.contains("search_index"));

        let mut mismatched_epoch: serde_json::Value = serde_json::from_slice(payload).unwrap();
        mismatched_epoch["results"][0]["selection"]["visible_epoch"] = serde_json::json!(3);
        assert!(
            project_search(&serde_json::to_vec(&mismatched_epoch).unwrap(), &expected()).is_err()
        );
    }

    #[test]
    fn v2_flat_hits_and_wrong_request_context_are_rejected() {
        let v2 = br#"{"schema_version":"resume-ir.search-response.v2","request_id":"synthetic-search-request","status":"ok","visible_epoch":4,"query_mode":"hybrid","partial":false,"partial_reasons":[],"latency_ms":4.0,"result_count":1,"results":[{"rank":1,"doc_id":"doc_00000000000000000000000000000000","version_id":"ver_00000000000000000000000000000000","file_name":"synthetic.pdf","snippet":"synthetic result"}]}"#;
        assert!(project_search(v2, &expected()).is_err());

        let empty_wrong_request = br#"{"schema_version":"resume-ir.search-response.v3","request_id":"other-request","status":"ok","visible_epoch":4,"query_mode":"hybrid","partial":false,"partial_reasons":[],"latency_ms":4.0,"result_count":0,"results":[]}"#;
        assert!(project_search(empty_wrong_request, &expected()).is_err());
    }
}
