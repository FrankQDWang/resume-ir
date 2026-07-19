use meta_store::ImportProcessingContract;

use super::{
    ImportOptions, ImportPipelineError, Result, OCR_PARSE_VERSION, PARSE_VERSION, SCHEMA_VERSION,
};

/// Returns the exact parser, derived-schema, and classifier contract used by
/// one import run.
///
/// Task coordinators must bind this contract before enqueueing a task. The
/// pipeline derives it again from the same options and fails closed when the
/// persisted binding differs.
pub fn current_import_processing_contract(
    options: &ImportOptions,
) -> Result<ImportProcessingContract> {
    let classifier_epoch = options
        .linear_promotion
        .classifier_epoch()
        .unwrap_or(meta_store::CLASSIFIER_EPOCH);
    ImportProcessingContract::new(
        PARSE_VERSION,
        OCR_PARSE_VERSION,
        SCHEMA_VERSION,
        classifier_epoch,
    )
    .map_err(ImportPipelineError::store)
}

#[cfg(test)]
#[path = "processing_contract_tests.rs"]
mod tests;
