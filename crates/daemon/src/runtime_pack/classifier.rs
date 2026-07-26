use std::path::Path;

use serde::Deserialize;

use crate::ipc::OptionalRuntimeReason;

use super::security::{
    ensure_not_cancelled, read_manifest_pinned_with_cancel, read_verified_file_with_cancel,
    validate_regular_file, PackFile,
};

const SCHEMA: &str = "resume-ir.desktop-classifier-model-pack.v1";
const EPOCH: &str = "precision_first_v4";
const FEATURE_CONTRACT: &str = "bounded_normalized_text_plus_structure_v1";
const SCOPE: &str = "user_authorized_internal_test";
const MODEL_FILE: &str = "linear-promotion-model.json";
const MANIFEST_SHA256: &str = "7e74014a3c021b8a1896cfaf3294e2f21e686516a1cb571d8dc9323bf4436b13";
const MODEL_BYTES: u64 = 20_942_422;
const MODEL_SHA256: &str = "3048fb78d27c0d96872f9d799e9e7a1195cfa517ff73b21031eac87b87cd443a";

pub(crate) struct ValidatedClassifierModel(Vec<u8>);

impl ValidatedClassifierModel {
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn from_bytes_for_test(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

#[cfg(test)]
pub(super) fn validate(model: &Path) -> Result<(), OptionalRuntimeReason> {
    validate_with_cancel(model, &|| false).map(|_| ())
}

pub(super) fn validate_with_cancel(
    model: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ValidatedClassifierModel, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let model = validate_regular_file(model)?;
    let root = model.parent().ok_or(OptionalRuntimeReason::Invalid)?;
    let manifest: Manifest = read_manifest_pinned_with_cancel(root, MANIFEST_SHA256, cancelled)?;
    if manifest.schema_version != SCHEMA
        || manifest.classifier_epoch != EPOCH
        || manifest.feature_contract != FEATURE_CONTRACT
        || manifest.distribution_scope != SCOPE
        || manifest.network_access != "disabled"
        || manifest.files.len() != 1
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let entry = manifest
        .files
        .first()
        .ok_or(OptionalRuntimeReason::Invalid)?;
    if entry.role != "linear_promotion_model"
        || entry.file != MODEL_FILE
        || entry.bytes != MODEL_BYTES
        || entry.sha256 != MODEL_SHA256
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let bytes =
        read_verified_file_with_cancel(&model, MODEL_BYTES, MODEL_SHA256, MODEL_BYTES, cancelled)?;
    ensure_not_cancelled(cancelled)?;
    Ok(ValidatedClassifierModel(bytes))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    classifier_epoch: String,
    feature_contract: String,
    distribution_scope: String,
    network_access: String,
    files: Vec<PackFile>,
}
