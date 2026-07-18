use std::path::{Path, PathBuf};

#[cfg(any(not(debug_assertions), test))]
use serde::Deserialize;
#[cfg(any(not(debug_assertions), test))]
use sha2::{Digest, Sha256};

use crate::daemon_client::DesktopError;

#[cfg(any(not(debug_assertions), test))]
use super::{EMBEDDING_PATH_MAX_BYTES, PACK_MANIFEST_MAX_BYTES};

#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_PACK_SCHEMA: &str = "resume-ir.desktop-classifier-model-pack.v1";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_EPOCH: &str = "precision_first_v4";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_FEATURE_CONTRACT: &str = "bounded_normalized_text_plus_structure_v1";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_DISTRIBUTION_SCOPE: &str = "user_authorized_internal_test";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_MODEL_ROLE: &str = "linear_promotion_model";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_MODEL_FILE: &str = "linear-promotion-model.json";
#[cfg(any(not(debug_assertions), test))]
const CLASSIFIER_MODEL_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct ClassifierRuntime {
    model_path: PathBuf,
}

impl ClassifierRuntime {
    pub(super) fn model_path(&self) -> &Path {
        &self.model_path
    }
}

#[cfg(debug_assertions)]
pub(super) fn configured_classifier_runtime(
    _resource_dir: &Path,
) -> Result<Option<ClassifierRuntime>, DesktopError> {
    Ok(None)
}

#[cfg(not(debug_assertions))]
pub(super) fn configured_classifier_runtime(
    resource_dir: &Path,
) -> Result<Option<ClassifierRuntime>, DesktopError> {
    resolve_packaged_classifier_runtime(resource_dir).map(Some)
}

#[cfg(any(not(debug_assertions), test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifierPackManifest {
    schema_version: String,
    classifier_epoch: String,
    feature_contract: String,
    distribution_scope: String,
    network_access: String,
    files: Vec<ClassifierPackFile>,
}

#[cfg(any(not(debug_assertions), test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifierPackFile {
    role: String,
    file: String,
    bytes: u64,
    sha256: String,
}

#[cfg(any(not(debug_assertions), test))]
fn resolve_packaged_classifier_runtime(
    resource_dir: &Path,
) -> Result<ClassifierRuntime, DesktopError> {
    let resource_dir = validate_classifier_directory(resource_dir)?;
    let manifest_path = resource_dir.join("runtime-pack.json");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|_| classifier_model_invalid())?;
    let manifest: ClassifierPackManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| classifier_model_invalid())?;
    if manifest.schema_version != CLASSIFIER_PACK_SCHEMA
        || manifest.classifier_epoch != CLASSIFIER_EPOCH
        || manifest.feature_contract != CLASSIFIER_FEATURE_CONTRACT
        || manifest.distribution_scope != CLASSIFIER_DISTRIBUTION_SCOPE
        || manifest.network_access != "disabled"
        || manifest.files.len() != 1
    {
        return Err(classifier_model_invalid());
    }
    let model = &manifest.files[0];
    if model.role != CLASSIFIER_MODEL_ROLE
        || model.file != CLASSIFIER_MODEL_FILE
        || model.bytes == 0
        || model.bytes > CLASSIFIER_MODEL_MAX_BYTES
        || model.sha256.len() != 64
        || !model
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(classifier_model_invalid());
    }
    let model_path = validate_classifier_file(resource_dir.join(CLASSIFIER_MODEL_FILE), model.bytes)?;
    let model_bytes = std::fs::read(&model_path).map_err(|_| classifier_model_invalid())?;
    if format!("{:x}", Sha256::digest(&model_bytes)) != model.sha256 {
        return Err(classifier_model_invalid());
    }
    Ok(ClassifierRuntime { model_path })
}

#[cfg(any(not(debug_assertions), test))]
fn validate_classifier_directory(path: &Path) -> Result<PathBuf, DesktopError> {
    let text = path.to_str().ok_or_else(classifier_model_invalid)?;
    if !path.is_absolute() || text.len() > EMBEDDING_PATH_MAX_BYTES {
        return Err(classifier_model_invalid());
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|_| classifier_model_invalid())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(classifier_model_invalid());
    }
    let canonical = path.canonicalize().map_err(|_| classifier_model_invalid())?;
    validate_classifier_file(canonical.join("runtime-pack.json"), 0)?;
    Ok(canonical)
}

#[cfg(any(not(debug_assertions), test))]
fn validate_classifier_file(path: PathBuf, expected_bytes: u64) -> Result<PathBuf, DesktopError> {
    let text = path.to_str().ok_or_else(classifier_model_invalid)?;
    if !path.is_absolute() || text.len() > EMBEDDING_PATH_MAX_BYTES {
        return Err(classifier_model_invalid());
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|_| classifier_model_invalid())?;
    let valid_size = if expected_bytes == 0 {
        metadata.len() > 0 && metadata.len() <= PACK_MANIFEST_MAX_BYTES
    } else {
        metadata.len() == expected_bytes
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || !valid_size {
        return Err(classifier_model_invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(classifier_model_invalid());
        }
    }
    path.canonicalize().map_err(|_| classifier_model_invalid())
}

#[cfg(any(not(debug_assertions), test))]
fn classifier_model_invalid() -> DesktopError {
    DesktopError::new(
        "classifier_model_invalid",
        "本地简历分类模型无效或不完整",
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn packaged_classifier_is_digest_bound() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-packaged-classifier-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let model_path = root.join(CLASSIFIER_MODEL_FILE);
        let model = br#"{"synthetic":"classifier"}"#;
        fs::write(&model_path, model).unwrap();
        let digest = format!("{:x}", Sha256::digest(model));
        fs::write(
            root.join("runtime-pack.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": CLASSIFIER_PACK_SCHEMA,
                "classifier_epoch": CLASSIFIER_EPOCH,
                "feature_contract": CLASSIFIER_FEATURE_CONTRACT,
                "distribution_scope": CLASSIFIER_DISTRIBUTION_SCOPE,
                "network_access": "disabled",
                "files": [{
                    "role": CLASSIFIER_MODEL_ROLE,
                    "file": CLASSIFIER_MODEL_FILE,
                    "bytes": model.len(),
                    "sha256": digest,
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let classifier = resolve_packaged_classifier_runtime(&root).unwrap();
        assert_eq!(classifier.model_path(), model_path.canonicalize().unwrap());
        fs::write(&model_path, br#"{"tampered":true}"#).unwrap();
        assert!(resolve_packaged_classifier_runtime(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
