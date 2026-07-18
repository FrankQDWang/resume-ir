use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    RuntimeError, DIMENSION, MAX_MANIFEST_BYTES, MODEL_ID, PACK_ID, PACK_SCHEMA, UPSTREAM_MODEL_ID,
    UPSTREAM_REVISION,
};

const UPSTREAM_MODEL_FILE: &str = "onnx/model_qint8_avx512_vnni.onnx";
const MAX_RUNTIME_LIBRARY_BYTES: u64 = 256 * 1024 * 1024;
const MODEL_ASSETS: [AssetIdentity; 5] = [
    AssetIdentity::new(
        FileRole::Model,
        118_346_824,
        "dd476dd0c2514e9b9be83aeb3853fac0763e0bdf4a71645407587d77c48a2d88",
    ),
    AssetIdentity::new(
        FileRole::Tokenizer,
        17_082_730,
        "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    ),
    AssetIdentity::new(
        FileRole::ModelConfig,
        655,
        "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
    ),
    AssetIdentity::new(
        FileRole::SpecialTokensMap,
        167,
        "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    ),
    AssetIdentity::new(
        FileRole::TokenizerConfig,
        443,
        "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    ),
];

pub(super) struct AssetIdentity {
    role: FileRole,
    bytes: u64,
    sha256: &'static str,
}

impl AssetIdentity {
    pub(super) const fn new(role: FileRole, bytes: u64, sha256: &'static str) -> Self {
        Self {
            role,
            bytes,
            sha256,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePackManifest {
    schema_version: String,
    runtime_pack_id: String,
    model_id: String,
    upstream_model_id: String,
    upstream_revision: String,
    upstream_model_file: String,
    quantization: String,
    dimension: usize,
    provider: String,
    network_access: String,
    license_reviewed: bool,
    model_license: String,
    onnxruntime_license: String,
    files: Vec<RuntimePackFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePackFile {
    role: FileRole,
    file: String,
    bytes: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Ord, PartialOrd, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum FileRole {
    RuntimeLibrary,
    Model,
    Tokenizer,
    ModelConfig,
    SpecialTokensMap,
    TokenizerConfig,
}

pub(super) struct RuntimePack {
    manifest: RuntimePackManifest,
    files: BTreeMap<FileRole, PathBuf>,
}

impl RuntimePack {
    pub(super) fn load(runtime_dir: &Path) -> Result<Self, RuntimeError> {
        Self::load_with_expected_model_assets(runtime_dir, &MODEL_ASSETS)
    }

    fn load_with_expected_model_assets(
        runtime_dir: &Path,
        expected_model_assets: &[AssetIdentity],
    ) -> Result<Self, RuntimeError> {
        if expected_model_assets.len() != 5 {
            return Err(RuntimeError::RuntimePackInvalid);
        }
        let root = canonical_directory(runtime_dir)?;
        let manifest_path = direct_regular_file(&root, "runtime-pack.json")?;
        if manifest_path
            .metadata()
            .map_err(|_| RuntimeError::RuntimePackInvalid)?
            .len()
            > MAX_MANIFEST_BYTES
        {
            return Err(RuntimeError::RuntimePackInvalid);
        }
        let manifest: RuntimePackManifest = serde_json::from_slice(
            &fs::read(&manifest_path).map_err(|_| RuntimeError::RuntimePackInvalid)?,
        )
        .map_err(|_| RuntimeError::RuntimePackInvalid)?;
        validate_manifest_identity(&manifest)?;

        let mut files = BTreeMap::new();
        for entry in &manifest.files {
            validate_file_name(&entry.file)?;
            let expected = expected_model_assets
                .iter()
                .find(|expected| expected.role == entry.role);
            let invalid_asset = match entry.role {
                FileRole::RuntimeLibrary => entry.bytes > MAX_RUNTIME_LIBRARY_BYTES,
                _ => expected.is_none_or(|expected| {
                    expected.bytes != entry.bytes || expected.sha256 != entry.sha256
                }),
            };
            if entry.bytes == 0 || !valid_digest(&entry.sha256) || invalid_asset {
                return Err(RuntimeError::RuntimePackInvalid);
            }
            let path = direct_regular_file(&root, &entry.file)?;
            let metadata = path
                .metadata()
                .map_err(|_| RuntimeError::RuntimePackInvalid)?;
            if metadata.len() != entry.bytes || sha256_file(&path)? != entry.sha256 {
                return Err(RuntimeError::RuntimePackInvalid);
            }
            if files.insert(entry.role, path).is_some() {
                return Err(RuntimeError::RuntimePackInvalid);
            }
        }
        if files.len() != 6
            || [
                FileRole::RuntimeLibrary,
                FileRole::Model,
                FileRole::Tokenizer,
                FileRole::ModelConfig,
                FileRole::SpecialTokensMap,
                FileRole::TokenizerConfig,
            ]
            .iter()
            .any(|role| !files.contains_key(role))
        {
            return Err(RuntimeError::RuntimePackInvalid);
        }
        Ok(Self { manifest, files })
    }

    #[cfg(test)]
    pub(super) fn load_with_expected_model_assets_for_test(
        runtime_dir: &Path,
        expected_model_assets: &[AssetIdentity],
    ) -> Result<Self, RuntimeError> {
        Self::load_with_expected_model_assets(runtime_dir, expected_model_assets)
    }

    pub(super) fn model_id(&self) -> &str {
        &self.manifest.model_id
    }

    pub(super) fn dimension(&self) -> usize {
        self.manifest.dimension
    }

    pub(super) fn file(&self, role: FileRole) -> Result<&Path, RuntimeError> {
        self.files
            .get(&role)
            .map(PathBuf::as_path)
            .ok_or(RuntimeError::RuntimePackInvalid)
    }

    #[cfg(test)]
    pub(super) fn file_count(&self) -> usize {
        self.files.len()
    }
}

fn validate_manifest_identity(manifest: &RuntimePackManifest) -> Result<(), RuntimeError> {
    if manifest.schema_version != PACK_SCHEMA
        || manifest.runtime_pack_id != PACK_ID
        || manifest.model_id != MODEL_ID
        || manifest.upstream_model_id != UPSTREAM_MODEL_ID
        || manifest.upstream_revision != UPSTREAM_REVISION
        || manifest.upstream_model_file != UPSTREAM_MODEL_FILE
        || manifest.quantization != "dynamic_int8"
        || manifest.dimension != DIMENSION
        || manifest.provider != "cpu"
        || manifest.network_access != "disabled"
        || !manifest.license_reviewed
        || !manifest.model_license.eq_ignore_ascii_case("MIT")
        || !manifest.onnxruntime_license.eq_ignore_ascii_case("MIT")
    {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn canonical_directory(path: &Path) -> Result<PathBuf, RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    path.canonicalize()
        .map_err(|_| RuntimeError::RuntimePackInvalid)
}

fn direct_regular_file(root: &Path, file: &str) -> Result<PathBuf, RuntimeError> {
    validate_file_name(file)?;
    let candidate = root.join(file);
    let metadata =
        fs::symlink_metadata(&candidate).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|_| RuntimeError::RuntimePackInvalid)?;
    if canonical.parent() != Some(root) {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    Ok(canonical)
}

fn validate_file_name(file: &str) -> Result<(), RuntimeError> {
    let path = Path::new(file);
    if file.is_empty()
        || file.len() > 128
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || !file
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-' | b'_'))
    {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let mut file = fs::File::open(path).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    Ok(format!("{:x}", digest.finalize()))
}
