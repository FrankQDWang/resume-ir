use std::path::Path;

use crate::ipc::OptionalRuntimeReason;

mod attestation;
mod classifier;
mod embedding;
mod macho;
mod macho_payload;
mod ocr;
mod security;

pub(crate) use attestation::validated_embedding_command;
pub(crate) use classifier::ValidatedClassifierModel;
pub(crate) use ocr::{
    validated_runtime as validated_ocr_runtime,
    validated_runtime_with_cancel as validated_ocr_runtime_with_cancel,
};

#[cfg(test)]
pub(crate) fn validate_embedding(
    command: &Path,
    model_id: &str,
    dimension: usize,
    runtime_dir: Option<&Path>,
) -> Result<(), OptionalRuntimeReason> {
    embedding::validate(command, model_id, dimension, runtime_dir)
}

pub(crate) fn validate_embedding_with_cancel(
    command: &Path,
    model_id: &str,
    dimension: usize,
    runtime_dir: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    embedding::validate_with_cancel(command, model_id, dimension, runtime_dir, cancelled)
}

#[cfg(test)]
pub(crate) fn validate_ocr(
    engine: &Path,
    renderer: Option<&Path>,
    requested_languages: &str,
    tessdata_dir: Option<&Path>,
) -> Result<(), OptionalRuntimeReason> {
    ocr::validate(engine, renderer, requested_languages, tessdata_dir)
}

#[cfg(test)]
pub(crate) fn validate_classifier(model: &Path) -> Result<(), OptionalRuntimeReason> {
    classifier::validate(model)
}

pub(crate) fn validate_classifier_with_cancel(
    model: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ValidatedClassifierModel, OptionalRuntimeReason> {
    classifier::validate_with_cancel(model, cancelled)
}

#[cfg(test)]
use attestation::{
    current_profile, current_target, validate_for_identity as validate_executable_for_identity,
    ExecutableIdentity, ExecutableRole,
};
#[cfg(test)]
use macho::payload_identity as executable_payload_identity;
#[cfg(test)]
use ocr::{
    validate_for_identity as validate_ocr_for_identity, windows_identity as windows_ocr_identity,
};
#[cfg(test)]
use security::{
    sha256_file, validate_pack_file_entries, validate_pack_file_entries_with_cancel, PackFile,
};

#[cfg(test)]
#[path = "runtime_pack_tests.rs"]
mod tests;
