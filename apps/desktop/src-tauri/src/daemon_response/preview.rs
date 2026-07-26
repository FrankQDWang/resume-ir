use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::daemon_client::DesktopError;
use crate::daemon_exchange::ExpectedResponse;

use super::{ensure, ensure_schema, MAX_SAFE_INTEGER};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    schema_version: String,
    request_id: String,
    status: String,
    lease_id: String,
    byte_size: u64,
    expires_in_ms: u64,
    range_bytes: usize,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RangeBody {
    schema_version: String,
    request_id: String,
    status: String,
    offset: u64,
    bytes_read: usize,
    total_bytes: u64,
    data_base64: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CloseBody {
    schema_version: String,
    request_id: String,
    status: String,
    closed: bool,
}

pub(super) fn project_preview(
    body: &[u8],
    expected: &ExpectedResponse,
) -> Result<serde_json::Value, DesktopError> {
    match expected {
        ExpectedResponse::PreviewCreate { request_id } => {
            let value: CreateBody = serde_json::from_slice(body).map_err(|_| protocol_error())?;
            ensure_schema(&value.schema_version, "resume-ir.source-preview.v1")?;
            ensure(
                value.request_id == *request_id
                    && value.status == "ok"
                    && valid_lease_id(&value.lease_id)
                    && (1..=MAX_SAFE_INTEGER).contains(&value.byte_size)
                    && (1..=120_000).contains(&value.expires_in_ms)
                    && value.range_bytes == 64 * 1024,
            )?;
            serde_json::to_value(value).map_err(|_| protocol_error())
        }
        ExpectedResponse::PreviewRange {
            request_id,
            offset,
            max_bytes,
        } => {
            let value: RangeBody = serde_json::from_slice(body).map_err(|_| protocol_error())?;
            ensure_schema(&value.schema_version, "resume-ir.source-preview-range.v1")?;
            ensure(
                value.request_id == *request_id
                    && value.status == "ok"
                    && value.offset == *offset
                    && value.total_bytes <= MAX_SAFE_INTEGER
                    && value.offset <= value.total_bytes
                    && value.bytes_read
                        == usize::try_from(
                            value
                                .total_bytes
                                .saturating_sub(value.offset)
                                .min(*max_bytes as u64),
                        )
                        .unwrap_or(usize::MAX)
                    && valid_base64_size(&value.data_base64, value.bytes_read),
            )?;
            serde_json::to_value(value).map_err(|_| protocol_error())
        }
        ExpectedResponse::PreviewClose { request_id } => {
            let value: CloseBody = serde_json::from_slice(body).map_err(|_| protocol_error())?;
            ensure_schema(&value.schema_version, "resume-ir.source-preview-close.v1")?;
            ensure(value.request_id == *request_id && value.status == "ok")?;
            serde_json::to_value(value).map_err(|_| protocol_error())
        }
        _ => Err(protocol_error()),
    }
}

fn valid_lease_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_base64_size(value: &str, decoded_bytes: usize) -> bool {
    if value.len() != decoded_bytes.div_ceil(3) * 4 {
        return false;
    }
    let mut decoded = Vec::with_capacity(decoded_bytes);
    base64::engine::general_purpose::STANDARD
        .decode_vec(value, &mut decoded)
        .is_ok()
        && decoded.len() == decoded_bytes
}

fn protocol_error() -> DesktopError {
    DesktopError::new("daemon_protocol", "daemon 响应合同无效")
}
