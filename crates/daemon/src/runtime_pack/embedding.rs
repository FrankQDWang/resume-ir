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
const PACK_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const UPSTREAM_ID: &str = "intfloat/multilingual-e5-small";
const UPSTREAM_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const MAC_MANIFEST_SHA256: &str =
    "a3f400c03a45d4213318ffd9f02a99018ae12d0e233d8bca467e0416382fee39";
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
    let root = canonical_input_directory(runtime_dir)?;
    ensure_not_cancelled(cancelled)?;
    let manifest: Manifest =
        read_manifest_pinned_with_cancel(&root, MAC_MANIFEST_SHA256, cancelled)?;
    if manifest.schema_version != SCHEMA
        || manifest.runtime_pack_id != PACK_ID
        || manifest.model_id != model_id
        || manifest.upstream_model_id != UPSTREAM_ID
        || manifest.upstream_revision != UPSTREAM_REVISION
        || manifest.upstream_model_file != "onnx/model_qint8_avx512_vnni.onnx"
        || manifest.quantization != "dynamic_int8"
        || manifest.dimension != dimension
        || manifest.provider != "cpu"
        || manifest.network_access != "disabled"
        || !manifest.license_reviewed
        || !manifest.model_license.eq_ignore_ascii_case("MIT")
        || !manifest.onnxruntime_license.eq_ignore_ascii_case("MIT")
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let files = validate_pack_files_with_cancel(&root, &manifest.files, cancelled)?;
    let roles = files.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_roles = [
        "runtime_library",
        "model",
        "tokenizer",
        "model_config",
        "special_tokens_map",
        "tokenizer_config",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if roles != expected_roles {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let runtime = files
        .get("runtime_library")
        .ok_or(OptionalRuntimeReason::Invalid)?;
    if runtime.bytes != MAC_RUNTIME_BYTES || runtime.sha256 != MAC_RUNTIME_SHA256 {
        return Err(OptionalRuntimeReason::Invalid);
    }
    for (role, bytes, digest) in MODEL_ASSETS {
        let entry = files.get(role).ok_or(OptionalRuntimeReason::Invalid)?;
        if entry.bytes != bytes || entry.sha256 != digest {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    ensure_not_cancelled(cancelled)?;
    Ok(())
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
    onnxruntime_license: String,
    files: Vec<PackFile>,
    upstream_model_file: String,
    quantization: String,
}
