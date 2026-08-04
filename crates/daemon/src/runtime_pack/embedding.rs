use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::ipc::OptionalRuntimeReason;

use super::attestation::validated_embedding_command;
use super::security::{
    canonical_input_directory, ensure_not_cancelled, read_manifest_pinned_with_cancel,
    validate_pack_files_with_cancel, PackFile,
};

const SCHEMA: &str = "resume-ir.embedding-runtime-pack.v1";
const COREML_SCHEMA: &str = "resume-ir.embedding-tokenizer-pack.v1";
const PACK_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const COREML_MODEL_ID: &str = "intfloat-multilingual-e5-small-coreml-fp16-r1";
const UPSTREAM_ID: &str = "intfloat/multilingual-e5-small";
const UPSTREAM_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const MAC_MANIFEST_SHA256: &str =
    "a3f400c03a45d4213318ffd9f02a99018ae12d0e233d8bca467e0416382fee39";
const COREML_TOKENIZER_MANIFEST_SHA256: &str =
    "c78d7c198fbc6f768ed0d3c7d0ed88f42d69be03cb68f6ff1e07610e64d7c860";
const MAC_RUNTIME_BYTES: u64 = 29_651_448;
const MAC_RUNTIME_SHA256: &str = "0d96dce50b9b3bf104857ce1c20352b9a268fab5b60e35cab613c0a8dd161c82";

const MODEL_ASSETS: [(&str, u64, &str); 5] = [
    (
        "model",
        118_346_824,
        "dd476dd0c2514e9b9be83aeb3853fac0763e0bdf4a71645407587d77c48a2d88",
    ),
    (
        "tokenizer",
        17_082_730,
        "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    ),
    (
        "model_config",
        655,
        "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
    ),
    (
        "special_tokens_map",
        167,
        "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    ),
    (
        "tokenizer_config",
        443,
        "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    ),
];

#[cfg(test)]
pub(super) fn validate(
    command: &Path,
    model_id: &str,
    dimension: usize,
    runtime_dir: Option<&Path>,
) -> Result<(), OptionalRuntimeReason> {
    validate_with_cancel(command, model_id, dimension, runtime_dir, &|| false)
}

pub(super) fn validate_with_cancel(
    command: &Path,
    model_id: &str,
    dimension: usize,
    runtime_dir: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    validated_embedding_command(command)?;
    ensure_not_cancelled(cancelled)?;
    if model_id.is_empty()
        || model_id.len() > 128
        || !model_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
        || !(1..=4096).contains(&dimension)
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let Some(runtime_dir) = runtime_dir else {
        return Err(OptionalRuntimeReason::Invalid);
    };
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let kind = PackKind::from_model_id(model_id).ok_or(OptionalRuntimeReason::Invalid)?;
    let root = canonical_input_directory(runtime_dir)?;
    ensure_not_cancelled(cancelled)?;
    let manifest: Manifest =
        read_manifest_pinned_with_cancel(&root, kind.manifest_sha256(), cancelled)?;
    if !kind.manifest_identity_valid(&manifest)
        || manifest.upstream_model_id != UPSTREAM_ID
        || manifest.upstream_revision != UPSTREAM_REVISION
        || manifest.dimension != dimension
        || manifest.network_access != "disabled"
        || !manifest.license_reviewed
        || !manifest.model_license.eq_ignore_ascii_case("MIT")
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let files = validate_pack_files_with_cancel(&root, &manifest.files, cancelled)?;
    let roles = files.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_roles = kind.roles().iter().copied().collect::<BTreeSet<_>>();
    if roles != expected_roles {
        return Err(OptionalRuntimeReason::Invalid);
    }
    if kind == PackKind::Onnx {
        let runtime = files
            .get("runtime_library")
            .ok_or(OptionalRuntimeReason::Invalid)?;
        if runtime.bytes != MAC_RUNTIME_BYTES || runtime.sha256 != MAC_RUNTIME_SHA256 {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    for (role, bytes, digest) in MODEL_ASSETS {
        if kind == PackKind::CoreMl && role == "model" {
            continue;
        }
        let entry = files.get(role).ok_or(OptionalRuntimeReason::Invalid)?;
        if entry.bytes != bytes || entry.sha256 != digest {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    ensure_not_cancelled(cancelled)?;
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PackKind {
    Onnx,
    CoreMl,
}

impl PackKind {
    fn from_model_id(model_id: &str) -> Option<Self> {
        match model_id {
            PACK_ID => Some(Self::Onnx),
            COREML_MODEL_ID => Some(Self::CoreMl),
            _ => None,
        }
    }

    fn manifest_sha256(self) -> &'static str {
        match self {
            Self::Onnx => MAC_MANIFEST_SHA256,
            Self::CoreMl => COREML_TOKENIZER_MANIFEST_SHA256,
        }
    }

    fn roles(self) -> &'static [&'static str] {
        const ONNX: [&str; 6] = [
            "runtime_library",
            "model",
            "tokenizer",
            "model_config",
            "special_tokens_map",
            "tokenizer_config",
        ];
        const COREML: [&str; 4] = [
            "tokenizer",
            "model_config",
            "special_tokens_map",
            "tokenizer_config",
        ];
        match self {
            Self::Onnx => &ONNX,
            Self::CoreMl => &COREML,
        }
    }

    fn manifest_identity_valid(self, manifest: &Manifest) -> bool {
        match self {
            Self::Onnx => {
                manifest.schema_version == SCHEMA
                    && manifest.runtime_pack_id == PACK_ID
                    && manifest.model_id == PACK_ID
                    && manifest.provider == "cpu"
                    && manifest.upstream_model_file.as_deref()
                        == Some("onnx/model_qint8_avx512_vnni.onnx")
                    && manifest.quantization.as_deref() == Some("dynamic_int8")
                    && manifest
                        .onnxruntime_license
                        .as_deref()
                        .is_some_and(|license| license.eq_ignore_ascii_case("MIT"))
            }
            Self::CoreMl => {
                manifest.schema_version == COREML_SCHEMA
                    && manifest.runtime_pack_id == COREML_MODEL_ID
                    && manifest.model_id == COREML_MODEL_ID
                    && manifest.provider == "coreml"
                    && manifest.upstream_model_file.is_none()
                    && manifest.quantization.is_none()
                    && manifest.onnxruntime_license.is_none()
            }
        }
    }
}

#[cfg(test)]
fn manifest_model_id(requested_model_id: &str) -> Option<&'static str> {
    match PackKind::from_model_id(requested_model_id) {
        Some(PackKind::Onnx) => Some(PACK_ID),
        Some(PackKind::CoreMl) => Some(COREML_MODEL_ID),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    runtime_pack_id: String,
    model_id: String,
    upstream_model_id: String,
    upstream_revision: String,
    dimension: usize,
    provider: String,
    network_access: String,
    license_reviewed: bool,
    model_license: String,
    onnxruntime_license: Option<String>,
    files: Vec<PackFile>,
    upstream_model_file: Option<String>,
    quantization: Option<String>,
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
