use resume_classifier::{CLASSIFIER_EPOCH, PROMOTED_EPOCH_PREFIX};

use crate::{
    contract_delta::{ContractDelta, ContractDeltaKind, ContractTransitionStrategy},
    ImportProcessingContract,
};

const PROMOTED_EPOCH: &str = "precision_first_v4_linear_0123456789ab";

fn contract(primary: &str, ocr: &str, derived: &str, classifier: &str) -> ImportProcessingContract {
    ImportProcessingContract::new(primary, ocr, derived, classifier).unwrap()
}

#[test]
fn identical_contracts_are_already_active() {
    let left = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        CLASSIFIER_EPOCH,
    );
    let right = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        CLASSIFIER_EPOCH,
    );
    let delta = ContractDelta::between(Some(&left), &right);
    assert_eq!(delta.kind, ContractDeltaKind::None);
    assert_eq!(delta.strategy, ContractTransitionStrategy::AlreadyActive);
}

#[test]
fn primary_parser_only_selects_pdf_root_rescan() {
    let committed = contract("parser-v1", "ocr-v1", "resume-ir-s9-v2", CLASSIFIER_EPOCH);
    let desired = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        CLASSIFIER_EPOCH,
    );
    let delta = ContractDelta::between(Some(&committed), &desired);
    assert_eq!(delta.kind, ContractDeltaKind::PrimaryParser);
    assert_eq!(delta.strategy, ContractTransitionStrategy::PdfRootRescan);
}

#[test]
fn classifier_epoch_selects_reclassify() {
    assert!(PROMOTED_EPOCH.starts_with(PROMOTED_EPOCH_PREFIX));
    let committed = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        CLASSIFIER_EPOCH,
    );
    let desired = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        PROMOTED_EPOCH,
    );
    let delta = ContractDelta::between(Some(&committed), &desired);
    assert_eq!(delta.kind, ContractDeltaKind::ClassifierEpoch);
    assert_eq!(
        delta.strategy,
        ContractTransitionStrategy::ClassifierReclassify
    );
}

#[test]
fn missing_committed_contract_takes_broadest_strategy() {
    let desired = contract(
        "parser-pdfium-v2",
        "ocr-v1",
        "resume-ir-s9-v2",
        CLASSIFIER_EPOCH,
    );
    let delta = ContractDelta::between(None, &desired);
    assert_eq!(delta.kind, ContractDeltaKind::Multiple);
    assert_eq!(delta.strategy, ContractTransitionStrategy::DerivedRebuild);
}

#[test]
fn multiple_changes_take_broadest_strategy() {
    let committed = contract("parser-v1", "ocr-v1", "resume-ir-s9-v2", CLASSIFIER_EPOCH);
    let desired = contract(
        "parser-pdfium-v2",
        "ocr-v2",
        "resume-ir-s9-v3",
        PROMOTED_EPOCH,
    );
    let delta = ContractDelta::between(Some(&committed), &desired);
    assert_eq!(delta.kind, ContractDeltaKind::Multiple);
    assert_eq!(delta.strategy, ContractTransitionStrategy::DerivedRebuild);
}
