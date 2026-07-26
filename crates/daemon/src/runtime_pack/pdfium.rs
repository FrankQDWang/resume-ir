use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::ipc::OptionalRuntimeReason;

use super::security::{
    canonical_input_directory, ensure_not_cancelled, read_manifest_with_cancel,
    validate_pack_files_with_cancel, PackFile,
};

const SCHEMA: &str = "resume-ir.pdfium-static-runtime-pack.v1";
const SOURCE_COMMIT: &str = "91b9d569b34be4f38eed7b3c49b227356c3aadad";
const BUILD_REVISION: &str = "f394ab2c993283e94680ca13db98b99927868e98";
const LICENSE_BYTES: u64 = 12_896;
const LICENSE_SHA256: &str = "1fe9dea718fbd75cf149adaf4d8a22a4335604d964ddb76d1b45383dec8668c9";

struct TargetContract {
    pack_id: &'static str,
    target: &'static str,
    arguments_bytes: u64,
    arguments_sha256: &'static str,
    source_contract_bytes: u64,
    source_contract_sha256: &'static str,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TARGET_CONTRACT: TargetContract = TargetContract {
    pack_id: "pdfium-chromium-7881-static-arm64-v1",
    target: "aarch64-apple-darwin",
    arguments_bytes: 376,
    arguments_sha256: "90035abcaaa04d163fc9f1e7af6c7163f3ff03d7998dc10edd8258694d478bd4",
    source_contract_bytes: 1_672,
    source_contract_sha256: "41eb0200b5ab7fe9143a045a70e9b34c53cb2db95e7a544b68fb6b6bb40ca581",
};

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const TARGET_CONTRACT: TargetContract = TargetContract {
    pack_id: "pdfium-chromium-7881-static-x64-v1",
    target: "x86_64-pc-windows-msvc",
    arguments_bytes: 298,
    arguments_sha256: "a9c705fe99e2a79cf69b67e6044d5eab1bb825cb8da93557982a58a82e5f0f11",
    source_contract_bytes: 3_036,
    source_contract_sha256: "70f88f9eb7df8e1eb489ec180800fb21b1ddc1be63fcac86a808909b6caeb97f",
};

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const TARGET_CONTRACT: TargetContract = TargetContract {
    pack_id: "",
    target: "",
    arguments_bytes: 0,
    arguments_sha256: "",
    source_contract_bytes: 0,
    source_contract_sha256: "",
};

pub(super) fn validate_with_cancel(
    runtime_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    if TARGET_CONTRACT.target.is_empty() {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let root = canonical_input_directory(runtime_dir)?;
    let manifest: Manifest = read_manifest_with_cancel(&root, cancelled)?;
    if manifest.schema_version != SCHEMA
        || manifest.runtime_pack_id != TARGET_CONTRACT.pack_id
        || manifest.target_triple != TARGET_CONTRACT.target
        || manifest.link_mode != "static"
        || manifest.source_commit != SOURCE_COMMIT
        || manifest.source_build_dependency_revision != BUILD_REVISION
        || manifest.product_runtime_network_access != "disabled"
        || manifest.files.len() != 3
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let expected = [
        ("license", "LICENSE", LICENSE_BYTES, LICENSE_SHA256),
        (
            "build_arguments",
            "args.gn",
            TARGET_CONTRACT.arguments_bytes,
            TARGET_CONTRACT.arguments_sha256,
        ),
        (
            "source_contract",
            "source-contract.json",
            TARGET_CONTRACT.source_contract_bytes,
            TARGET_CONTRACT.source_contract_sha256,
        ),
    ];
    if !manifest
        .files
        .iter()
        .zip(expected)
        .all(|(entry, (role, file, bytes, sha256))| {
            entry.role == role
                && entry.file == file
                && entry.bytes == bytes
                && entry.sha256 == sha256
        })
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    validate_pack_files_with_cancel(&root, &manifest.files, cancelled)?;
    let mut names = fs::read_dir(&root)
        .map_err(|_| OptionalRuntimeReason::Invalid)?
        .map(|entry| {
            entry
                .map_err(|_| OptionalRuntimeReason::Invalid)
                .and_then(|entry| {
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| OptionalRuntimeReason::Invalid)
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names
        != [
            "LICENSE",
            "args.gn",
            "runtime-pack.json",
            "source-contract.json",
        ]
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    ensure_not_cancelled(cancelled)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    runtime_pack_id: String,
    target_triple: String,
    link_mode: String,
    source_commit: String,
    source_build_dependency_revision: String,
    product_runtime_network_access: String,
    files: Vec<PackFile>,
}
