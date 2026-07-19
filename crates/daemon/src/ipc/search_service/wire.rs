use std::net::TcpStream;

use crate::query_timing::{QueryStage, QueryStageTiming};
use crate::search_command::{DaemonSearchOutput, SearchCommandCompletion};

use super::admission::ClientClass;
use super::batch;
use super::cancellation::CancelStatus;

const REQUEST_SCHEMA_VERSION: &str = "resume-ir.ipc-request.v3";
const RESPONSE_SCHEMA_VERSION: &str = "resume-ir.search-response.v3";
const REQUEST_ID_MAX_BYTES: usize = 128;
const CANCEL_TOKEN_MAX_BYTES: usize = 128;
pub(super) const DEADLINE_MS_MAX: u64 = 60_000;

pub(crate) struct RequestEnvelope {
    pub(crate) request_id: String,
    pub(crate) deadline_ms: u64,
    pub(crate) payload: serde_json::Value,
    pub(super) cancel_token: Option<String>,
    pub(super) client_class: ClientClass,
}

impl RequestEnvelope {
    pub(crate) fn cancel_token(&self) -> Option<&str> {
        self.cancel_token.as_deref()
    }

    pub(crate) fn client_class(&self) -> ClientClass {
        self.client_class
    }
}

pub(crate) struct CancelRequest {
    pub(super) request_id: String,
    pub(super) cancel_token: String,
}

pub(super) enum SearchReply {
    Single {
        stream: TcpStream,
        completion: crate::ipc::ConnectionCompletion,
    },
    Batch(batch::BatchChildReply),
}

impl SearchReply {
    pub(super) fn try_clone(&self) -> crate::Result<Self> {
        match self {
            Self::Single { stream, completion } => stream
                .try_clone()
                .map(|stream| Self::Single {
                    stream,
                    completion: completion.clone(),
                })
                .map_err(|_| {
                    crate::DaemonError::recoverable_dependency("unable to monitor query deadline")
                }),
            Self::Batch(reply) => Ok(Self::Batch(reply.clone())),
        }
    }

    pub(super) fn write_output(&mut self, output: DaemonSearchOutput) -> crate::Result<()> {
        let body = search_output_body(&output);
        match self {
            Self::Single { stream, completion } => {
                let server_timing = output.stage_timing.server_timing_header_value();
                let result =
                    crate::ipc::response::write_search_response(stream, &server_timing, &body)
                        .map_err(crate::DaemonError::response_sink);
                complete_connection(completion, &result);
                result
            }
            Self::Batch(reply) => {
                reply.complete(200, &body);
                Ok(())
            }
        }
    }

    pub(super) fn write_error(
        &mut self,
        request_id: &str,
        status_code: u16,
        code: &str,
        message: &str,
    ) -> crate::Result<()> {
        let body = error_body(request_id, code, message);
        match self {
            Self::Single { stream, completion } => {
                let result = crate::ipc::response::write_http_response(
                    stream,
                    status_code,
                    "application/json",
                    &body,
                )
                .map_err(crate::DaemonError::response_sink);
                complete_connection(completion, &result);
                result
            }
            Self::Batch(reply) => {
                reply.complete(status_code, &body);
                Ok(())
            }
        }
    }

    pub(super) fn write_overloaded(&mut self, request_id: &str) -> crate::Result<()> {
        let body = super::overload_body(request_id);
        match self {
            Self::Single { stream, completion } => {
                let result = crate::ipc::response::write_http_response(
                    stream,
                    503,
                    "application/json",
                    &body,
                )
                .map_err(crate::DaemonError::response_sink);
                complete_connection(completion, &result);
                result
            }
            Self::Batch(reply) => {
                reply.complete(503, &body);
                Ok(())
            }
        }
    }
}

fn complete_connection(completion: &crate::ipc::ConnectionCompletion, result: &crate::Result<()>) {
    let outcome = match result {
        Ok(()) => crate::ipc::ConnectionOutcome::Completed,
        Err(error) => crate::ipc::ConnectionOutcome::from_request_result(Err(
            crate::ipc::RequestFailure::from(error),
        )),
    };
    completion.finish(outcome);
}

pub(crate) fn parse_request(body: &[u8]) -> Result<RequestEnvelope, &'static str> {
    let value = serde_json::from_slice::<serde_json::Value>(body).map_err(|_| "invalid json")?;
    let object = value
        .as_object()
        .ok_or("search request must be an object")?;
    const ALLOWED_FIELDS: &[&str] = &[
        "schema_version",
        "request_id",
        "client_capability",
        "deadline_ms",
        "cancel_token",
        "payload",
    ];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err("search request contains an unknown field");
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(REQUEST_SCHEMA_VERSION)
    {
        return Err("search request schema_version is invalid");
    }
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|request_id| valid_opaque_id(request_id))
        .ok_or("request_id is invalid")?
        .to_string();
    let client_capability = value
        .get("client_capability")
        .and_then(serde_json::Value::as_str)
        .ok_or("client_capability is invalid")?;
    let client_class =
        ClientClass::parse(client_capability).ok_or("client_capability is invalid")?;
    let deadline_ms = value
        .get("deadline_ms")
        .and_then(serde_json::Value::as_u64)
        .filter(|deadline_ms| (1..=DEADLINE_MS_MAX).contains(deadline_ms))
        .ok_or("deadline_ms is invalid")?;
    let payload = value
        .get("payload")
        .filter(|payload| payload.is_object())
        .cloned()
        .ok_or("payload must be an object")?;
    let cancel_token = value
        .get("cancel_token")
        .map(|value| {
            value
                .as_str()
                .filter(|token| valid_cancel_token(token))
                .map(str::to_string)
                .ok_or("cancel_token is invalid")
        })
        .transpose()?;
    Ok(RequestEnvelope {
        request_id,
        deadline_ms,
        payload,
        cancel_token,
        client_class,
    })
}

pub(crate) fn parse_cancel_request(body: &[u8]) -> Result<CancelRequest, &'static str> {
    let value = serde_json::from_slice::<serde_json::Value>(body).map_err(|_| "invalid json")?;
    let object = value
        .as_object()
        .ok_or("search cancel request must be an object")?;
    const ALLOWED_FIELDS: &[&str] = &["schema_version", "request_id", "cancel_token"];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err("search cancel request contains an unknown field");
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some("resume-ir.search-cancel-request.v1")
    {
        return Err("search cancel schema_version is invalid");
    }
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .filter(|request_id| valid_opaque_id(request_id))
        .ok_or("request_id is invalid")?
        .to_string();
    let cancel_token = value
        .get("cancel_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| valid_cancel_token(token))
        .ok_or("cancel_token is invalid")?
        .to_string();
    Ok(CancelRequest {
        request_id,
        cancel_token,
    })
}

fn search_output_body(output: &DaemonSearchOutput) -> String {
    let results = output
        .hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "rank": hit.rank,
                "selection": {
                    "doc_id": hit.selection.document_id.as_str(),
                    "version_id": hit.selection.resume_version_id.as_str(),
                    "visible_epoch": hit.selection.visible_epoch,
                },
                "file_name": hit.file_name,
                "snippet": hit.snippet,
            })
        })
        .collect::<Vec<_>>();
    let (status, search_index) = match output.completion {
        SearchCommandCompletion::Complete => ("ok", "available"),
        SearchCommandCompletion::Cancelled => ("cancelled", "not_observed"),
    };
    response_body(SearchResponse {
        request_id: output.request_id.clone(),
        status,
        visible_epoch: output.visible_epoch,
        query_mode: output.mode.response_label(),
        partial_reasons: output.partial_reasons.clone(),
        latency_ms: output.elapsed.as_secs_f64() * 1_000.0,
        stage_latency_ms: stage_latency_json(&output.stage_timing),
        search_index,
        results,
    })
}

fn stage_latency_json(stage_timing: &QueryStageTiming) -> serde_json::Value {
    serde_json::json!({
        "parse": stage_timing.duration_ms(QueryStage::QueryParse),
        "prefilter": stage_timing.duration_ms(QueryStage::Prefilter),
        "bm25": stage_timing.duration_ms(QueryStage::Bm25),
        "ann": stage_timing.duration_ms(QueryStage::Ann),
        "fusion": stage_timing.duration_ms(QueryStage::Fusion),
        "bulk_hydrate": stage_timing.duration_ms(QueryStage::BulkHydrate),
        "snippet": stage_timing.duration_ms(QueryStage::Snippet),
    })
}

struct SearchResponse {
    request_id: String,
    status: &'static str,
    visible_epoch: u64,
    query_mode: &'static str,
    partial_reasons: Vec<&'static str>,
    latency_ms: f64,
    stage_latency_ms: serde_json::Value,
    search_index: &'static str,
    results: Vec<serde_json::Value>,
}

fn response_body(response: SearchResponse) -> String {
    let result_count = response.results.len();
    serde_json::json!({
        "schema_version": RESPONSE_SCHEMA_VERSION,
        "request_id": response.request_id,
        "status": response.status,
        "visible_epoch": response.visible_epoch,
        "query_mode": response.query_mode,
        "partial": !response.partial_reasons.is_empty(),
        "partial_reasons": response.partial_reasons,
        "latency_ms": response.latency_ms,
        "stage_latency_ms": response.stage_latency_ms,
        "search_index": response.search_index,
        "result_count": result_count,
        "results": response.results,
    })
    .to_string()
}

pub(crate) fn error_body(request_id: &str, code: &str, _message: &str) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v1",
        "request_id": request_id,
        "status": "error",
        "error": {
            "code": code,
            "action": error_action(code),
        },
    })
    .to_string()
}

pub(crate) fn overload_body(request_id: &str) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v1",
        "request_id": request_id,
        "status": "error",
        "error": {
            "code": "OVERLOADED",
            "action": "retry",
            "retry_after_ms": 250,
        },
    })
    .to_string()
}

fn error_action(code: &str) -> &'static str {
    match code {
        "BAD_REQUEST" => "correct_request",
        "CONFLICT" => "retry",
        "NOT_FOUND" => "refresh_search",
        "LIMIT_EXCEEDED" => "reduce_page_size",
        "SEMANTIC_DISABLED" => "select_supported_mode",
        "REPAIRING" => "wait_for_repair",
        "QUERY_SERVICE_UNAVAILABLE" => "retry",
        _ => "retry",
    }
}

pub(super) fn cancel_response_body(request_id: &str, status: CancelStatus) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.search-cancel-response.v1",
        "request_id": request_id,
        "status": status.label(),
    })
    .to_string()
}

pub(crate) fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= REQUEST_ID_MAX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_cancel_token(value: &str) -> bool {
    value.len() <= CANCEL_TOKEN_MAX_BYTES && valid_opaque_id(value)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
