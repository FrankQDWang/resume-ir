use super::sealed_source_disposition_count;
use crate::ImportSummary;

#[test]
fn frozen_pdfs_remain_in_scan_truth_but_not_in_the_sealed_manifest() {
    let summary = ImportSummary {
        files_discovered: 3,
        deferred_pdf_documents: 1,
        ..ImportSummary::default()
    };

    assert_eq!(summary.files_discovered, 3);
    assert_eq!(sealed_source_disposition_count(&summary), 2);
}

#[test]
fn deferred_count_cannot_underflow_a_corrupt_in_memory_summary() {
    let summary = ImportSummary {
        files_discovered: 1,
        deferred_pdf_documents: 2,
        ..ImportSummary::default()
    };

    assert_eq!(sealed_source_disposition_count(&summary), 0);
}
