use crate::{
    observe_writer_contract_transition, ImportProcessingContract, WriterContractTransitionOutcome,
    CLASSIFIER_EPOCH,
};

fn contract(primary: &str) -> ImportProcessingContract {
    ImportProcessingContract::new(primary, "ocr-v1", "resume-ir-s9-v2", CLASSIFIER_EPOCH).unwrap()
}

#[test]
fn observe_marks_identical_contracts_already_active() {
    let desired = contract("parser-pdfium-v2");
    let (_, outcome) = observe_writer_contract_transition(Some(&desired), &desired, 0);
    assert_eq!(outcome, WriterContractTransitionOutcome::AlreadyActive);
}

#[test]
fn observe_requires_transition_when_primary_parser_changes() {
    let committed = contract("parser-v1");
    let desired = contract("parser-pdfium-v2");
    let (_, outcome) = observe_writer_contract_transition(Some(&committed), &desired, 0);
    assert_eq!(outcome, WriterContractTransitionOutcome::TransitionRequired);
}

#[test]
fn observe_blocks_on_running_owner_before_commit() {
    let committed = contract("parser-v1");
    let desired = contract("parser-pdfium-v2");
    let (_, outcome) = observe_writer_contract_transition(Some(&committed), &desired, 2);
    assert_eq!(
        outcome,
        WriterContractTransitionOutcome::BlockedByRunningOwner
    );
}
