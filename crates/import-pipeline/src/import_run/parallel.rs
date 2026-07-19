use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Instant;

use fs_crawler::DiscoveredFile;
use meta_store::{ImportTaskId, OwnedMetaStore, UnixTimestamp};
use sectionizer::Sectionizer;

use super::scheduler::{
    commit_ready_import_file_results, finish_import_file, process_files_sequential,
    process_indexed_files_sequential,
};
use crate::file_processing::{
    drain_available_parse_results, insert_import_file_result, insert_parse_result,
    parse_worker_loop, prepare_file_for_parse, process_file, recv_parse_result_with_cancel_poll,
    send_parse_work_with_backpressure, ImportFileResult, ParseWorkItem, ParseWorkResult,
    ParseWorkerClock, PendingSearchableDocument, PreparedFile,
};
use crate::publication_coordinator::PendingProjectionRemovals;
use crate::search_artifact_cache::CurrentImportDocumentCache;
use crate::source_dispositions::ImportDispositionBatches;
use crate::{
    ImportCancelCheckPhase, ImportParseWorkers, ImportSummary, LinearPromotionPolicy, Result,
    SearchPublicationVectorization,
};

pub(super) fn process_files_with_parse_workers(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    files: Vec<DiscoveredFile>,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    disposition_batches: &mut ImportDispositionBatches,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    parse_workers: ImportParseWorkers,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    let total_files = files.len();
    let worker_count = parse_workers.get().min(total_files);
    if worker_count <= 1 {
        summary
            .worker_metrics
            .record_parse_worker_count(worker_count);
        let sectionizer = Sectionizer::default();
        return process_files_sequential(
            data_dir,
            store,
            task_id,
            files,
            &sectionizer,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
            linear_promotion,
        );
    }

    let sectionizer = Sectionizer::default();
    let mut remaining_files = Vec::new();
    let mut indexed_files = files.into_iter().enumerate();
    for (index, file) in &mut indexed_files {
        if summary.searchable_documents > 0 {
            remaining_files.push((index, file));
            break;
        }
        set_cancel_phase(ImportCancelCheckPhase::SequentialParse);
        ensure_not_cancelled()?;
        let processed = process_file(
            data_dir,
            store,
            &file,
            &sectionizer,
            now,
            ensure_not_cancelled,
            &mut summary.stage_timings,
            &mut summary.worker_metrics,
            &mut summary.content_bytes_read,
            linear_promotion,
        )?;
        finish_import_file(
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
            index,
            total_files,
            &file,
            processed,
        )?;
    }
    remaining_files.extend(indexed_files);
    if remaining_files.is_empty() {
        summary.worker_metrics.record_parse_worker_count(1);
        return Ok(());
    }

    let worker_count = parse_workers.get().min(remaining_files.len());
    summary
        .worker_metrics
        .record_parse_worker_count(worker_count);
    if worker_count <= 1 {
        return process_indexed_files_sequential(
            data_dir,
            store,
            task_id,
            remaining_files,
            &sectionizer,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            total_files,
            search_vectorization,
            linear_promotion,
        );
    }

    let mut parse_worker_clock = ParseWorkerClock::default();
    thread::scope(|scope| -> Result<()> {
        let (work_tx, work_rx) = mpsc::sync_channel::<ParseWorkItem>(worker_count);
        let work_rx = Arc::new(Mutex::new(work_rx));
        let (result_tx, result_rx) = mpsc::sync_channel::<ParseWorkResult>(worker_count);

        for _ in 0..worker_count {
            let work_rx = Arc::clone(&work_rx);
            let result_tx = result_tx.clone();
            let linear_promotion = linear_promotion.clone();
            scope.spawn(move || parse_worker_loop(work_rx, result_tx, &linear_promotion));
        }
        drop(result_tx);

        let mut pending_results = BTreeMap::<usize, ImportFileResult>::new();
        let mut next_commit_index = remaining_files[0].0;

        for (index, file) in remaining_files.into_iter() {
            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            drain_available_parse_results(&result_rx, &mut pending_results)?;
            commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                disposition_batches,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
                search_vectorization,
                linear_promotion,
            )?;

            set_cancel_phase(ImportCancelCheckPhase::ParsePrepare);
            let prepared = prepare_file_for_parse(
                store,
                index,
                file,
                now,
                ensure_not_cancelled,
                &mut summary.stage_timings.db,
                &mut summary.worker_metrics.parse_prepare,
                &mut summary.content_bytes_read,
                linear_promotion,
            )?;
            match prepared {
                PreparedFile::Ready(result) => {
                    insert_import_file_result(
                        &mut pending_results,
                        index,
                        ImportFileResult::Processed(result),
                    )?;
                }
                PreparedFile::Parse(work) => {
                    send_parse_work_with_backpressure(
                        &work_tx,
                        &result_rx,
                        &mut pending_results,
                        &mut summary.worker_metrics,
                        ensure_not_cancelled,
                        set_cancel_phase,
                        work,
                    )?;
                }
            }

            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            drain_available_parse_results(&result_rx, &mut pending_results)?;
            commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                disposition_batches,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
                search_vectorization,
                linear_promotion,
            )?;
        }

        drop(work_tx);
        while next_commit_index < total_files {
            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            if commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                disposition_batches,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
                search_vectorization,
                linear_promotion,
            )? {
                continue;
            }

            set_cancel_phase(ImportCancelCheckPhase::ParseResultWait);
            let wait_started = Instant::now();
            let result = recv_parse_result_with_cancel_poll(&result_rx, ensure_not_cancelled)?;
            summary.worker_metrics.parse_result_wait += wait_started.elapsed();
            insert_parse_result(&mut pending_results, result)?;
        }

        Ok(())
    })?;

    summary.worker_metrics.record_parse_worker_timing(
        parse_worker_clock.active_elapsed(),
        parse_worker_clock.worker_wall_elapsed(),
    );
    summary.stage_timings.parse += parse_worker_clock.worker_wall_elapsed();

    Ok(())
}
