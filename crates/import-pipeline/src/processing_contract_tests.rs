use meta_store::{ImportProcessingContract, CLASSIFIER_EPOCH};

use super::*;

#[test]
fn current_import_contract_binds_every_processing_identity() {
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();

    assert_eq!(contract.primary_parse_version(), PARSE_VERSION);
    assert_eq!(contract.ocr_parse_version(), OCR_PARSE_VERSION);
    assert_eq!(contract.derived_schema_version(), "resume-ir-s9-v2");
    assert_eq!(contract.derived_schema_version(), SCHEMA_VERSION);
    assert_eq!(contract.classifier_epoch(), CLASSIFIER_EPOCH);
}

#[test]
fn bounded_mention_schema_has_a_new_processing_identity() {
    let current = current_import_processing_contract(&ImportOptions::default()).unwrap();
    let previous = ImportProcessingContract::new(
        PARSE_VERSION,
        OCR_PARSE_VERSION,
        "resume-ir-s9-v1",
        CLASSIFIER_EPOCH,
    )
    .unwrap();

    assert_ne!(current.id(), previous.id());
}
