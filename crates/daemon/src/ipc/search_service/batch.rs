use std::collections::HashSet;
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{BatchAdmissionPermit, RequestEnvelope};

const REQUEST_SCHEMA_VERSION: &str = "resume-ir.search-batch-request.v1";
const CHILD_RESPONSE_SCHEMA_VERSION: &str = "resume-ir.search-batch-child-response.v1";
const MAX_BATCH_QUERIES: usize = 64;

pub(crate) struct BatchRequest {
    pub(crate) batch_id: String,
    pub(crate) requests: Vec<RequestEnvelope>,
}

pub(crate) fn parse_request(body: &[u8]) -> Result<BatchRequest, &'static str> {
    let value = serde_json::from_slice::<serde_json::Value>(body).map_err(|_| "invalid json")?;
    let object = value
        .as_object()
        .ok_or("search batch request must be an object")?;
    const ALLOWED_FIELDS: &[&str] = &["schema_version", "batch_id", "requests"];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err("search batch request contains an unknown field");
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(REQUEST_SCHEMA_VERSION)
    {
        return Err("search batch schema_version is invalid");
    }
    let batch_id = value
        .get("batch_id")
        .and_then(serde_json::Value::as_str)
        .filter(|batch_id| super::valid_opaque_id(batch_id))
        .ok_or("batch_id is invalid")?
        .to_string();
    let requests = value
        .get("requests")
        .and_then(serde_json::Value::as_array)
        .filter(|requests| !requests.is_empty() && requests.len() <= MAX_BATCH_QUERIES)
        .ok_or("requests must contain 1..=64 child requests")?;

    let mut parsed = Vec::with_capacity(requests.len());
    let mut request_ids = HashSet::with_capacity(requests.len());
    let mut cancel_tokens = HashSet::with_capacity(requests.len());
    for request in requests {
        let envelope = super::parse_request(
            serde_json::to_string(request)
                .map_err(|_| "child request is invalid")?
                .as_bytes(),
        )?;
        if !request_ids.insert(envelope.request_id.clone()) {
            return Err("child request_id values must be unique");
        }
        if let Some(cancel_token) = envelope.cancel_token() {
            if !cancel_tokens.insert(cancel_token.to_string()) {
                return Err("child cancel_token values must be unique");
            }
        }
        if parsed
            .first()
            .is_some_and(|first: &RequestEnvelope| first.client_class() != envelope.client_class())
        {
            return Err("child client_capability values must match");
        }
        parsed.push(envelope);
    }

    Ok(BatchRequest {
        batch_id,
        requests: parsed,
    })
}

pub(crate) struct BatchWriter {
    inner: Arc<BatchWriterInner>,
}

struct BatchWriterInner {
    stream: Mutex<TcpStream>,
    batch_id: String,
    remaining: AtomicUsize,
    admission: Mutex<Option<BatchAdmissionPermit>>,
    completion: crate::ipc::ConnectionCompletion,
    response_failure: Mutex<Option<crate::ipc::ResponseSinkError>>,
}

impl BatchWriter {
    pub(crate) fn start(
        mut stream: TcpStream,
        batch_id: String,
        child_count: usize,
        admission: BatchAdmissionPermit,
        completion: crate::ipc::ConnectionCompletion,
    ) -> crate::Result<Self> {
        crate::ipc::response::write_all(
            &mut stream,
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
        )
        .map_err(crate::DaemonError::response_sink)?;
        Ok(Self {
            inner: Arc::new(BatchWriterInner {
                stream: Mutex::new(stream),
                batch_id,
                remaining: AtomicUsize::new(child_count),
                admission: Mutex::new(Some(admission)),
                completion,
                response_failure: Mutex::new(None),
            }),
        })
    }

    pub(crate) fn child(&self, sequence: usize, request_id: String) -> BatchChildReply {
        BatchChildReply {
            writer: Arc::clone(&self.inner),
            sequence,
            request_id,
            completed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn finish_failure(&self, failure: crate::ipc::RequestFailure) {
        self.inner
            .completion
            .finish(crate::ipc::ConnectionOutcome::from_request_result(Err(
                failure,
            )));
    }
}

#[derive(Clone)]
pub(crate) struct BatchChildReply {
    writer: Arc<BatchWriterInner>,
    sequence: usize,
    request_id: String,
    completed: Arc<AtomicBool>,
}

impl BatchChildReply {
    pub(crate) fn complete(&self, status_code: u16, response_body: &str) {
        if self
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let response =
            serde_json::from_str::<serde_json::Value>(response_body).unwrap_or_else(|_| {
                serde_json::json!({
                    "schema_version": "resume-ir.error.v2",
                    "request_id": self.request_id,
                    "status": "error",
                    "error": {
                        "code": "QUERY_SERVICE_UNAVAILABLE",
                        "action": "repair_required",
                        "capability": serde_json::Value::Null,
                        "reason": serde_json::Value::Null,
                    },
                })
            });
        let line = serde_json::json!({
            "schema_version": CHILD_RESPONSE_SCHEMA_VERSION,
            "batch_id": self.writer.batch_id,
            "sequence": self.sequence,
            "http_status": status_code,
            "response": response,
        })
        .to_string();
        let response_failure = if let Ok(mut stream) = self.writer.stream.lock() {
            crate::ipc::response::write_all(&mut stream, line.as_bytes())
                .and_then(|()| crate::ipc::response::write_all(&mut stream, b"\n"))
                .and_then(|()| crate::ipc::response::flush(&mut stream))
                .err()
        } else {
            Some(crate::ipc::ResponseSinkError::Unavailable)
        };
        if let Some(error) = response_failure {
            if let Ok(mut failure) = self.writer.response_failure.lock() {
                failure.get_or_insert(error);
            }
        }
        if self.writer.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            if let Ok(stream) = self.writer.stream.lock() {
                let _ = stream.shutdown(Shutdown::Write);
            }
            if let Ok(mut admission) = self.writer.admission.lock() {
                admission.take();
            }
            let outcome = self
                .writer
                .response_failure
                .lock()
                .ok()
                .and_then(|failure| *failure)
                .map_or(crate::ipc::ConnectionOutcome::Completed, |error| {
                    crate::ipc::ConnectionOutcome::from_request_result(Err(
                        crate::ipc::RequestFailure::ResponseSink(error),
                    ))
                });
            self.writer.completion.finish(outcome);
        }
    }
}

pub(crate) fn overload_body(batch_id: &str) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "request_id": batch_id,
        "status": "error",
        "error": {
            "code": "OVERLOADED",
            "action": "retry",
            "retry_after_ms": 250,
            "capability": serde_json::Value::Null,
            "reason": serde_json::Value::Null,
        },
    })
    .to_string()
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
