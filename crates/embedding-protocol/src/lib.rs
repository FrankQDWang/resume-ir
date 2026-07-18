//! Bounded typed protocol shared by the resident embedding runtime and its owner.

use std::fmt;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "resume-ir.embedding-stream.v1";
pub const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_INPUTS: usize = 4;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_MODEL_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingRole {
    Query,
    Passage,
}

#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidentInput {
    pub role: EmbeddingRole,
    pub text: String,
}

impl fmt::Debug for ResidentInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResidentInput")
            .field("role", &self.role)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedRequest {
    pub schema_version: String,
    pub request_id: u64,
    pub model_id: String,
    pub dimension: usize,
    pub inputs: Vec<ResidentInput>,
}

impl EmbedRequest {
    pub fn new(
        request_id: u64,
        model_id: impl Into<String>,
        dimension: usize,
        inputs: Vec<ResidentInput>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            request_id,
            model_id: model_id.into(),
            dimension,
            inputs,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.schema_version, &self.model_id, self.dimension)?;
        if self.inputs.is_empty() || self.inputs.len() > MAX_INPUTS {
            return Err(ProtocolError::InvalidPayload);
        }
        if self
            .inputs
            .iter()
            .any(|input| input.text.len() > MAX_TEXT_BYTES)
        {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }
}

impl fmt::Debug for EmbedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbedRequest")
            .field("schema_version", &self.schema_version)
            .field("request_id", &self.request_id)
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .field("input_count", &self.inputs.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentErrorCode {
    InvalidRequest,
    IdentityMismatch,
    InferenceFailed,
    OutputInvalid,
    RuntimeUnavailable,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResidentResponse {
    Ready {
        schema_version: String,
        model_id: String,
        dimension: usize,
    },
    Result {
        schema_version: String,
        request_id: u64,
        vectors: Vec<Vec<f32>>,
    },
    Error {
        schema_version: String,
        request_id: Option<u64>,
        code: ResidentErrorCode,
        retryable: bool,
    },
}

impl ResidentResponse {
    pub fn ready(model_id: impl Into<String>, dimension: usize) -> Self {
        Self::Ready {
            schema_version: SCHEMA_VERSION.to_string(),
            model_id: model_id.into(),
            dimension,
        }
    }

    pub fn result(request_id: u64, vectors: Vec<Vec<f32>>) -> Self {
        Self::Result {
            schema_version: SCHEMA_VERSION.to_string(),
            request_id,
            vectors,
        }
    }

    pub fn error(request_id: Option<u64>, code: ResidentErrorCode, retryable: bool) -> Self {
        Self::Error {
            schema_version: SCHEMA_VERSION.to_string(),
            request_id,
            code,
            retryable,
        }
    }

    pub fn validate_ready(&self, model_id: &str, dimension: usize) -> Result<(), ProtocolError> {
        match self {
            Self::Ready {
                schema_version,
                model_id: actual_model_id,
                dimension: actual_dimension,
            } => {
                validate_identity(schema_version, actual_model_id, *actual_dimension)?;
                if actual_model_id != model_id || *actual_dimension != dimension {
                    return Err(ProtocolError::InvalidPayload);
                }
                Ok(())
            }
            _ => Err(ProtocolError::InvalidPayload),
        }
    }

    pub fn validate_result(
        &self,
        request_id: u64,
        input_count: usize,
        dimension: usize,
    ) -> Result<(), ProtocolError> {
        match self {
            Self::Result {
                schema_version,
                request_id: actual_request_id,
                vectors,
            } => {
                if schema_version != SCHEMA_VERSION
                    || *actual_request_id != request_id
                    || vectors.len() != input_count
                    || vectors.iter().any(|vector| {
                        vector.len() != dimension || vector.iter().any(|value| !value.is_finite())
                    })
                {
                    return Err(ProtocolError::InvalidPayload);
                }
                Ok(())
            }
            _ => Err(ProtocolError::InvalidPayload),
        }
    }
}

impl fmt::Debug for ResidentResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready {
                schema_version,
                model_id,
                dimension,
            } => formatter
                .debug_struct("Ready")
                .field("schema_version", schema_version)
                .field("model_id", model_id)
                .field("dimension", dimension)
                .finish(),
            Self::Result {
                schema_version,
                request_id,
                vectors,
            } => formatter
                .debug_struct("Result")
                .field("schema_version", schema_version)
                .field("request_id", request_id)
                .field("vector_count", &vectors.len())
                .finish(),
            Self::Error {
                schema_version,
                request_id,
                code,
                retryable,
            } => formatter
                .debug_struct("Error")
                .field("schema_version", schema_version)
                .field("request_id", request_id)
                .field("code", code)
                .field("retryable", retryable)
                .finish(),
        }
    }
}

fn validate_identity(
    schema_version: &str,
    model_id: &str,
    dimension: usize,
) -> Result<(), ProtocolError> {
    if schema_version != SCHEMA_VERSION
        || model_id.is_empty()
        || model_id.len() > MAX_MODEL_ID_BYTES
        || model_id.bytes().any(|byte| byte.is_ascii_control())
        || dimension == 0
    {
        return Err(ProtocolError::InvalidPayload);
    }
    Ok(())
}

pub enum ProtocolError {
    Io(io::Error),
    FrameTooLarge,
    InvalidPayload,
}

impl fmt::Debug for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "ProtocolError::Io(<redacted>)",
            Self::FrameTooLarge => "ProtocolError::FrameTooLarge",
            Self::InvalidPayload => "ProtocolError::InvalidPayload",
        })
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "embedding stream I/O failed",
            Self::FrameTooLarge => "embedding stream frame exceeded its bound",
            Self::InvalidPayload => "embedding stream payload is invalid",
        })
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::FrameTooLarge | Self::InvalidPayload => None,
        }
    }
}

pub fn write_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    max_bytes: usize,
) -> Result<(), ProtocolError> {
    let mut payload = BoundedBuffer::new(max_bytes.min(u32::MAX as usize));
    serde_json::to_writer(&mut payload, value).map_err(|error| {
        if error.is_io() {
            ProtocolError::FrameTooLarge
        } else {
            ProtocolError::InvalidPayload
        }
    })?;
    let payload = payload.into_inner();
    if payload.is_empty() {
        return Err(ProtocolError::FrameTooLarge);
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .map_err(ProtocolError::Io)?;
    writer.write_all(&payload).map_err(ProtocolError::Io)?;
    writer.flush().map_err(ProtocolError::Io)
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl BoundedBuffer {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_bytes,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn read_frame<T: DeserializeOwned>(
    reader: &mut impl Read,
    max_bytes: usize,
) -> Result<Option<T>, ProtocolError> {
    let mut prefix = [0_u8; 4];
    let first = read_prefix_byte(reader)?;
    let Some(first) = first else {
        return Ok(None);
    };
    prefix[0] = first;
    reader
        .read_exact(&mut prefix[1..])
        .map_err(ProtocolError::Io)?;
    let size = u32::from_be_bytes(prefix) as usize;
    if size == 0 || size > max_bytes {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut payload = vec![0_u8; size];
    reader.read_exact(&mut payload).map_err(ProtocolError::Io)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|_| ProtocolError::InvalidPayload)
}

fn read_prefix_byte(reader: &mut impl Read) -> Result<Option<u8>, ProtocolError> {
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(byte[0])),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ProtocolError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_is_framed_and_redacted() {
        let request = EmbedRequest::new(
            7,
            "model-r1",
            3,
            vec![ResidentInput {
                role: EmbeddingRole::Query,
                text: "private synthetic query".to_string(),
            }],
        );
        request.validate().expect("request validates");
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request, MAX_REQUEST_BYTES).expect("frame writes");
        let decoded = read_frame::<EmbedRequest>(&mut bytes.as_slice(), MAX_REQUEST_BYTES)
            .expect("frame reads")
            .expect("request exists");
        assert_eq!(decoded, request);
        assert!(!format!("{request:?}").contains("private synthetic query"));
    }

    #[test]
    fn frame_reader_rejects_zero_oversized_truncated_and_unknown_payloads() {
        assert!(matches!(
            read_frame::<EmbedRequest>(&mut [0, 0, 0, 0].as_slice(), MAX_REQUEST_BYTES),
            Err(ProtocolError::FrameTooLarge)
        ));
        let too_large = ((MAX_REQUEST_BYTES + 1) as u32).to_be_bytes();
        assert!(matches!(
            read_frame::<EmbedRequest>(&mut too_large.as_slice(), MAX_REQUEST_BYTES),
            Err(ProtocolError::FrameTooLarge)
        ));
        assert!(matches!(
            read_frame::<EmbedRequest>(&mut [0, 0, 0, 2, b'{'].as_slice(), MAX_REQUEST_BYTES),
            Err(ProtocolError::Io(_))
        ));
        let payload = br#"{"schema_version":"resume-ir.embedding-stream.v1","request_id":1,"model_id":"m","dimension":1,"inputs":[],"extra":true}"#;
        let mut unknown = (payload.len() as u32).to_be_bytes().to_vec();
        unknown.extend_from_slice(payload);
        assert!(matches!(
            read_frame::<EmbedRequest>(&mut unknown.as_slice(), MAX_REQUEST_BYTES),
            Err(ProtocolError::InvalidPayload)
        ));
    }

    #[test]
    fn result_validation_rejects_wrong_request_and_non_finite_vectors() {
        let wrong = ResidentResponse::result(2, vec![vec![1.0]]);
        assert!(wrong.validate_result(1, 1, 1).is_err());
        let invalid = ResidentResponse::result(1, vec![vec![f32::NAN]]);
        assert!(invalid.validate_result(1, 1, 1).is_err());
        assert!(!format!("{invalid:?}").contains("NaN"));
    }
}
