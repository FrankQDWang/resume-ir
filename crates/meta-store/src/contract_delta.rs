//! ContractDelta planner for online processing-contract transitions.

use crate::ImportProcessingContract;

/// Closed set of processing-contract field changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractDeltaKind {
    None,
    PrimaryParser,
    OcrParser,
    DerivedSchema,
    ClassifierEpoch,
    Multiple,
    Unknown,
}

/// Strategy selected from a ContractDelta.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractTransitionStrategy {
    AlreadyActive,
    PdfRootRescan,
    OcrRequeue,
    ClassifierReclassify,
    DerivedRebuild,
    Unsupported,
}

/// Diff between committed and desired processing contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractDelta {
    pub kind: ContractDeltaKind,
    pub strategy: ContractTransitionStrategy,
    pub primary_changed: bool,
    pub ocr_changed: bool,
    pub derived_changed: bool,
    pub classifier_changed: bool,
}

impl ContractDelta {
    /// Computes the delta and selects the broadest applicable strategy.
    pub fn between(
        committed: Option<&ImportProcessingContract>,
        desired: &ImportProcessingContract,
    ) -> Self {
        let Some(committed) = committed else {
            // No committed writer contract: all dimensions change; take the
            // broadest strategy (derived rebuild), never a narrow PDF-only path.
            return Self {
                kind: ContractDeltaKind::Multiple,
                strategy: ContractTransitionStrategy::DerivedRebuild,
                primary_changed: true,
                ocr_changed: true,
                derived_changed: true,
                classifier_changed: true,
            };
        };
        if committed.id() == desired.id() {
            return Self {
                kind: ContractDeltaKind::None,
                strategy: ContractTransitionStrategy::AlreadyActive,
                primary_changed: false,
                ocr_changed: false,
                derived_changed: false,
                classifier_changed: false,
            };
        }

        let primary_changed = committed.primary_parse_version() != desired.primary_parse_version();
        let ocr_changed = committed.ocr_parse_version() != desired.ocr_parse_version();
        let derived_changed =
            committed.derived_schema_version() != desired.derived_schema_version();
        let classifier_changed = committed.classifier_epoch() != desired.classifier_epoch();
        let changed = [
            primary_changed,
            ocr_changed,
            derived_changed,
            classifier_changed,
        ]
        .into_iter()
        .filter(|changed| *changed)
        .count();

        if changed == 0 {
            // Distinct contract ids with identical fields is an invariant break.
            return Self {
                kind: ContractDeltaKind::Unknown,
                strategy: ContractTransitionStrategy::Unsupported,
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            };
        }

        if changed > 1 {
            return Self {
                kind: ContractDeltaKind::Multiple,
                strategy: broadest_strategy(
                    primary_changed,
                    ocr_changed,
                    derived_changed,
                    classifier_changed,
                ),
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            };
        }

        if primary_changed {
            Self {
                kind: ContractDeltaKind::PrimaryParser,
                strategy: ContractTransitionStrategy::PdfRootRescan,
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            }
        } else if ocr_changed {
            Self {
                kind: ContractDeltaKind::OcrParser,
                strategy: ContractTransitionStrategy::OcrRequeue,
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            }
        } else if classifier_changed {
            Self {
                kind: ContractDeltaKind::ClassifierEpoch,
                strategy: ContractTransitionStrategy::ClassifierReclassify,
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            }
        } else {
            Self {
                kind: ContractDeltaKind::DerivedSchema,
                strategy: ContractTransitionStrategy::DerivedRebuild,
                primary_changed,
                ocr_changed,
                derived_changed,
                classifier_changed,
            }
        }
    }
}

fn broadest_strategy(
    primary_changed: bool,
    ocr_changed: bool,
    derived_changed: bool,
    classifier_changed: bool,
) -> ContractTransitionStrategy {
    // Broadest first: derived rebuild ⊃ classifier reclassify ⊃ OCR ⊃ PDF rescan.
    if derived_changed {
        ContractTransitionStrategy::DerivedRebuild
    } else if classifier_changed {
        ContractTransitionStrategy::ClassifierReclassify
    } else if ocr_changed {
        ContractTransitionStrategy::OcrRequeue
    } else if primary_changed {
        ContractTransitionStrategy::PdfRootRescan
    } else {
        ContractTransitionStrategy::Unsupported
    }
}

#[cfg(test)]
#[path = "contract_delta_tests.rs"]
mod tests;
