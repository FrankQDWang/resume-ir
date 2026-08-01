use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    RuntimeError, DIMENSION, MAX_RUNTIME_PATH_BYTES, UPSTREAM_MODEL_ID, UPSTREAM_REVISION,
};

pub(super) const ARTIFACT_EXPERIMENT_MANIFEST_ENV: &str =
    "RESUME_IR_EMBEDDING_ARTIFACT_EXPERIMENT_MANIFEST";
const MANIFEST_SCHEMA: &str = "resume-ir.embedding-artifact-experiment.v1";
const CALIBRATION_ID: &str = "public_synthetic_100_minmax_matmul_v1";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_MODEL_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ArtifactVariant {
    Fp32,
    StaticQdqS8s8,
    StaticQoperatorU8s8,
}

impl ArtifactVariant {
    fn expected_model_id(self) -> &'static str {
        match self {
            Self::Fp32 => "intfloat-multilingual-e5-small-fp32-exp-r1",
            Self::StaticQdqS8s8 => "intfloat-multilingual-e5-small-static-qdq-s8s8-exp-r1",
            Self::StaticQoperatorU8s8 => {
                "intfloat-multilingual-e5-small-static-qoperator-u8s8-exp-r1"
            }
        }
    }

    fn expected_quantization(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Fp32 => ("none", "fp32", "float32", "float32"),
            Self::StaticQdqS8s8 => ("static", "qdq", "qint8", "qint8"),
            Self::StaticQoperatorU8s8 => ("static", "qoperator", "quint8", "qint8"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentManifest {
    schema_version: String,
    variant: ArtifactVariant,
    model_id: String,
    upstream_model_id: String,
    upstream_revision: String,
    dimension: usize,
    generator_onnx_version: String,
    generator_ort_version: String,
    quantization: QuantizationIdentity,
    model: ModelIdentity,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantizationIdentity {
    method: String,
    format: String,
    activation_type: String,
    weight_type: String,
    calibration_id: Option<String>,
    op_types: Vec<String>,
    graph_optimization: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelIdentity {
    file: String,
    bytes: u64,
    sha256: String,
}

pub(super) struct ArtifactExperiment {
    model_id: String,
    dimension: usize,
    model_path: PathBuf,
}

impl ArtifactExperiment {
    pub(super) fn load_from_environment() -> Result<Self, RuntimeError> {
        let value = std::env::var_os(ARTIFACT_EXPERIMENT_MANIFEST_ENV)
            .ok_or(RuntimeError::EnvironmentInvalid)?;
        Self::load_environment_value(&value)
    }

    fn load_environment_value(value: &OsStr) -> Result<Self, RuntimeError> {
        let path = PathBuf::from(value);
        let text = path.to_str().ok_or(RuntimeError::EnvironmentInvalid)?;
        if !path.is_absolute() || text.len() > MAX_RUNTIME_PATH_BYTES {
            return Err(RuntimeError::EnvironmentInvalid);
        }
        Self::load(&path)
    }

    fn load(manifest_path: &Path) -> Result<Self, RuntimeError> {
        let parent = direct_owner_only_parent(manifest_path)?;
        let manifest_metadata =
            fs::symlink_metadata(manifest_path).map_err(|_| RuntimeError::RuntimePackInvalid)?;
        if !manifest_metadata.is_file()
            || manifest_metadata.file_type().is_symlink()
            || manifest_metadata.len() == 0
            || manifest_metadata.len() > MAX_MANIFEST_BYTES
        {
            return Err(RuntimeError::RuntimePackInvalid);
        }
        let manifest: ExperimentManifest = serde_json::from_slice(
            &fs::read(manifest_path).map_err(|_| RuntimeError::RuntimePackInvalid)?,
        )
        .map_err(|_| RuntimeError::RuntimePackInvalid)?;
        validate_manifest(&manifest)?;
        validate_file_name(&manifest.model.file)?;
        let model_path = parent.join(&manifest.model.file);
        let model_metadata =
            fs::symlink_metadata(&model_path).map_err(|_| RuntimeError::RuntimePackInvalid)?;
        if !model_metadata.is_file()
            || model_metadata.file_type().is_symlink()
            || model_metadata.len() != manifest.model.bytes
            || model_metadata.len() == 0
            || model_metadata.len() > MAX_MODEL_BYTES
            || sha256_file(&model_path)? != manifest.model.sha256
        {
            return Err(RuntimeError::RuntimePackInvalid);
        }
        Ok(Self {
            model_id: manifest.model_id,
            dimension: manifest.dimension,
            model_path,
        })
    }

    pub(super) fn model_id(&self) -> &str {
        &self.model_id
    }

    pub(super) fn dimension(&self) -> usize {
        self.dimension
    }

    pub(super) fn model_path(&self) -> &Path {
        &self.model_path
    }
}

fn validate_manifest(manifest: &ExperimentManifest) -> Result<(), RuntimeError> {
    let (method, format, activation, weight) = manifest.variant.expected_quantization();
    let calibration_valid = if manifest.variant == ArtifactVariant::Fp32 {
        manifest.quantization.calibration_id.is_none()
    } else {
        manifest.quantization.calibration_id.as_deref() == Some(CALIBRATION_ID)
    };
    if manifest.schema_version != MANIFEST_SCHEMA
        || manifest.model_id != manifest.variant.expected_model_id()
        || manifest.upstream_model_id != UPSTREAM_MODEL_ID
        || manifest.upstream_revision != UPSTREAM_REVISION
        || manifest.dimension != DIMENSION
        || manifest.generator_onnx_version != "1.19.0"
        || manifest.generator_ort_version != "1.27.0"
        || manifest.quantization.method != method
        || manifest.quantization.format != format
        || manifest.quantization.activation_type != activation
        || manifest.quantization.weight_type != weight
        || !calibration_valid
        || manifest.quantization.op_types != ["MatMul"]
        || manifest.quantization.graph_optimization != "disabled"
        || manifest.model.bytes == 0
        || manifest.model.bytes > MAX_MODEL_BYTES
        || !valid_digest(&manifest.model.sha256)
    {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    Ok(())
}

fn direct_owner_only_parent(path: &Path) -> Result<PathBuf, RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    let parent = path.parent().ok_or(RuntimeError::RuntimePackInvalid)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(RuntimeError::RuntimePackInvalid);
        }
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| RuntimeError::RuntimePackInvalid)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| RuntimeError::RuntimePackInvalid)?;
    if path.file_name().is_none() || canonical_path.parent() != Some(canonical_parent.as_path()) {
        return Err(RuntimeError::RuntimePackInvalid);
    }
    Ok(canonical_parent)
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

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let mut file = fs::File::open(path).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).map_err(|_| RuntimeError::RuntimePackInvalid)?;
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
#[path = "artifact_experiment_tests.rs"]
mod tests;
