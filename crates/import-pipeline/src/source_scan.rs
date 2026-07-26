use meta_store::{
    ImportProcessingContract, ImportTaskId, OwnedMetaStore, ScanCounts, ScanPhase, UnixTimestamp,
};

use crate::ImportSummary;

/// Completes the current source-root scan after one import task succeeds.
///
/// Legacy CLI-only imports without a registered source root remain outside the
/// source-truth model. Current managed roots always publish their scan result
/// before another scan can be coordinated.
pub fn finish_source_scan_success(
    store: &OwnedMetaStore,
    canonical_root_path: &str,
    task_id: &ImportTaskId,
    processing_contract: &ImportProcessingContract,
    summary: &ImportSummary,
    finished_at: UnixTimestamp,
) -> meta_store::Result<()> {
    let Some(root) = store.source_root_by_canonical_path(canonical_root_path)? else {
        return Ok(());
    };
    let Some(snapshot) = store
        .latest_scan_snapshot(&root.id)?
        .filter(|snapshot| snapshot.id == task_id.as_str() && snapshot.phase.is_active())
    else {
        return Ok(());
    };
    let elapsed = finished_at
        .as_unix_seconds()
        .saturating_sub(snapshot.started_at.as_unix_seconds());
    let processed = summary.processed_documents;
    let rate = (elapsed > 0 && processed > 0).then_some(processed as f64 / elapsed as f64);
    let classifications = store
        .source_root_classification_counts(&root.id, processing_contract.classifier_epoch())?;
    let counts = ScanCounts {
        discovered: summary.files_discovered as u64,
        searchable: summary.searchable_documents as u64,
        non_resume: classifications.non_resume,
        needs_review: classifications.needs_review,
        ocr: summary.ocr_required_documents as u64,
        failed: summary.failed_documents as u64,
        ignored: summary.ignored_entries as u64,
        processed: processed as u64,
        total: Some(summary.files_discovered as u64),
        errors: summary.scan_errors as u64,
    };
    let scan_is_complete = summary.source_truth_complete
        && summary.scan_errors == 0
        && !summary.scan_budget.is_some_and(|budget| budget.exhausted);
    if scan_is_complete {
        store.reconcile_complete_source_scan(
            &root.id,
            task_id.as_str(),
            counts,
            rate,
            finished_at,
        )?;
        if summary.deferred_pdf_documents == 0 {
            store.complete_pdf_reprocess_root(
                &root.id,
                task_id,
                processing_contract.primary_parse_version(),
                finished_at,
            )?;
        } else {
            store.requeue_pdf_reprocess_root(
                &root.id,
                task_id,
                processing_contract.primary_parse_version(),
                finished_at,
            )?;
        }
    } else {
        store.fail_or_partial_scan(
            &root.id,
            task_id.as_str(),
            counts,
            ScanPhase::Partial,
            finished_at,
        )?;
        store.requeue_pdf_reprocess_root(
            &root.id,
            task_id,
            processing_contract.primary_parse_version(),
            finished_at,
        )?;
    }
    Ok(())
}

/// Closes the current source-root scan after one import task fails.
pub fn finish_source_scan_failure(
    store: &OwnedMetaStore,
    canonical_root_path: &str,
    task_id: &ImportTaskId,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> meta_store::Result<()> {
    let Some(root) = store.source_root_by_canonical_path(canonical_root_path)? else {
        return Ok(());
    };
    let Some(snapshot) = store
        .latest_scan_snapshot(&root.id)?
        .filter(|snapshot| snapshot.id == task_id.as_str() && snapshot.phase.is_active())
    else {
        return Ok(());
    };
    store.fail_or_partial_scan(
        &root.id,
        task_id.as_str(),
        snapshot.counts,
        ScanPhase::Failed,
        now,
    )?;
    store.requeue_pdf_reprocess_root(
        &root.id,
        task_id,
        processing_contract.primary_parse_version(),
        now,
    )?;
    Ok(())
}
