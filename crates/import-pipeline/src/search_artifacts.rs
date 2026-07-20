use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use index_fulltext::{
    incremental_snapshot_documents, publish_snapshot_with_control,
    publish_trusted_redacted_snapshot_with_control, IndexDocument, PublishedSnapshotMetadata,
    SnapshotPublishControl, SnapshotPublishPhase,
};
use meta_store::{
    ActiveSearchProjection, Document, MigrationRebuildProjectionRow, OwnedMetaStore, ResumeVersion,
    SearchPublicationSession, UnixTimestamp,
};
use sectionizer::Sectionizer;

use super::search_artifact_cache::{
    apply_document_delta, ensure_cache_matches_generation, CurrentImportCacheMode,
    CurrentImportDocumentCache,
};
use super::search_publication::{
    load_migration_rebuild_publication_base, load_search_publication_base, projections_after_delta,
    run_search_publication_transaction, SearchPublicationBase, SearchPublicationDecision,
    SearchPublicationTransactionOutcome, SearchPublicationView,
};
#[cfg(test)]
use super::search_publication::{
    prepare_search_publication_for_test, PreparedSearchPublicationForTest,
};
use super::search_publication_vector::staged_search_version_texts;
use super::{
    sections_to_index, ImportCancelCheckPhase, ImportPipelineError, ImportResourcePolicy, Result,
    SearchPublicationVectorization,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_incremental_search_artifacts(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    replacement_documents: Vec<IndexDocument>,
    removed_document_ids: &BTreeSet<String>,
    ocr_required_documents: usize,
    deleted_documents: usize,
    current_import_cache: Option<&mut CurrentImportDocumentCache>,
    current_import_cache_mode: CurrentImportCacheMode,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
    vectorization: &SearchPublicationVectorization,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> Result<SearchPublicationDecision>,
) -> Result<SearchPublicationTransactionOutcome> {
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    let base = load_search_publication_base(store)?;
    let index_root = data_dir.join("search-index");
    let staged_version_texts =
        staged_search_version_texts(&replacement_documents, ensure_not_cancelled)?;
    let next_projections = projections_after_delta(
        &base.projections,
        &replacement_documents,
        removed_document_ids,
    )?;
    if let Some(current_import_cache) = current_import_cache {
        ensure_cache_matches_generation(
            &index_root,
            base.generation.as_deref(),
            current_import_cache,
        )?;
        apply_document_delta(
            &mut current_import_cache.documents,
            replacement_documents,
            removed_document_ids,
        );
        let indexed_documents = current_import_cache.documents.len();
        let generation = search_generation_token(
            now,
            indexed_documents,
            ocr_required_documents,
            deleted_documents,
        );
        let sectionizer = Sectionizer::default();
        let publication = run_search_publication_transaction(
            publication_session,
            now,
            classifier_epoch,
            base,
            &generation,
            next_projections,
            &staged_version_texts,
            vectorization,
            ensure_not_cancelled,
            || match current_import_cache_mode {
                CurrentImportCacheMode::Retain => write_cached_fulltext_snapshot(
                    data_dir,
                    &generation,
                    &current_import_cache.documents,
                    &sectionizer,
                    ensure_not_cancelled,
                    set_cancel_phase,
                    record_phase_timing,
                    index_writer_heap_bytes,
                ),
                CurrentImportCacheMode::Consume => write_cached_fulltext_snapshot_consuming(
                    data_dir,
                    &generation,
                    &mut current_import_cache.documents,
                    &sectionizer,
                    ensure_not_cancelled,
                    set_cancel_phase,
                    record_phase_timing,
                    index_writer_heap_bytes,
                ),
            },
            decide,
        )?;
        if let Some(committed) = publication.committed() {
            current_import_cache.base_generation =
                Some(committed.fulltext.generation().to_string());
        }
        return Ok(publication);
    }

    let index_documents = incremental_snapshot_documents(
        &index_root,
        base.generation.as_deref(),
        replacement_documents,
        removed_document_ids,
    )
    .map_err(ImportPipelineError::index)?;
    let indexed_documents = index_documents.len();
    let generation = search_generation_token(
        now,
        indexed_documents,
        ocr_required_documents,
        deleted_documents,
    );
    run_search_publication_transaction(
        publication_session,
        now,
        classifier_epoch,
        base,
        &generation,
        next_projections,
        &staged_version_texts,
        vectorization,
        ensure_not_cancelled,
        || {
            write_fulltext_snapshot(
                data_dir,
                &generation,
                index_documents,
                FullTextSnapshotInput::NeedsRedaction,
                None,
                None,
                None,
                index_writer_heap_bytes,
            )
        },
        decide,
    )
}

/// Creates an intentionally unterminated publication for restart/recovery
/// tests. Production callers must use `publish_incremental_search_artifacts`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn write_incremental_search_artifacts_for_test<'session>(
    publication_session: &'session SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    replacement_documents: Vec<IndexDocument>,
    removed_document_ids: &BTreeSet<String>,
    ocr_required_documents: usize,
    deleted_documents: usize,
    current_import_cache: Option<&mut CurrentImportDocumentCache>,
    current_import_cache_mode: CurrentImportCacheMode,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
    vectorization: &SearchPublicationVectorization,
) -> Result<PreparedSearchPublicationForTest<'session>> {
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    let base = load_search_publication_base(store)?;
    let index_root = data_dir.join("search-index");
    let staged_version_texts =
        staged_search_version_texts(&replacement_documents, ensure_not_cancelled)?;
    let next_projections = projections_after_delta(
        &base.projections,
        &replacement_documents,
        removed_document_ids,
    )?;
    if let Some(current_import_cache) = current_import_cache {
        ensure_cache_matches_generation(
            &index_root,
            base.generation.as_deref(),
            current_import_cache,
        )?;
        apply_document_delta(
            &mut current_import_cache.documents,
            replacement_documents,
            removed_document_ids,
        );
        let indexed_documents = current_import_cache.documents.len();
        let generation = search_generation_token(
            now,
            indexed_documents,
            ocr_required_documents,
            deleted_documents,
        );
        let sectionizer = Sectionizer::default();
        let publication = prepare_search_publication_for_test(
            publication_session,
            now,
            classifier_epoch,
            base,
            &generation,
            next_projections,
            &staged_version_texts,
            vectorization,
            ensure_not_cancelled,
            || match current_import_cache_mode {
                CurrentImportCacheMode::Retain => write_cached_fulltext_snapshot(
                    data_dir,
                    &generation,
                    &current_import_cache.documents,
                    &sectionizer,
                    ensure_not_cancelled,
                    set_cancel_phase,
                    record_phase_timing,
                    index_writer_heap_bytes,
                ),
                CurrentImportCacheMode::Consume => write_cached_fulltext_snapshot_consuming(
                    data_dir,
                    &generation,
                    &mut current_import_cache.documents,
                    &sectionizer,
                    ensure_not_cancelled,
                    set_cancel_phase,
                    record_phase_timing,
                    index_writer_heap_bytes,
                ),
            },
        )?;
        current_import_cache.base_generation = Some(publication.generation().to_string());
        return Ok(publication);
    }

    let index_documents = incremental_snapshot_documents(
        &index_root,
        base.generation.as_deref(),
        replacement_documents,
        removed_document_ids,
    )
    .map_err(ImportPipelineError::index)?;
    let indexed_documents = index_documents.len();
    let generation = search_generation_token(
        now,
        indexed_documents,
        ocr_required_documents,
        deleted_documents,
    );
    prepare_search_publication_for_test(
        publication_session,
        now,
        classifier_epoch,
        base,
        &generation,
        next_projections,
        &staged_version_texts,
        vectorization,
        ensure_not_cancelled,
        || {
            write_fulltext_snapshot(
                data_dir,
                &generation,
                index_documents,
                FullTextSnapshotInput::NeedsRedaction,
                None,
                None,
                None,
                index_writer_heap_bytes,
            )
        },
    )
}

pub(super) fn publish_rebuilt_search_artifacts(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    pending_document_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
    vectorization: &SearchPublicationVectorization,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> Result<SearchPublicationDecision>,
) -> Result<SearchPublicationTransactionOutcome> {
    let store = publication_session.owned_store();
    let base = load_search_publication_base(store)?;
    publish_rebuilt_search_artifacts_from_base(
        publication_session,
        now,
        classifier_epoch,
        pending_document_ids,
        pending_index_documents,
        base,
        vectorization,
        None,
        decide,
    )
}

pub(super) fn publish_migration_rebuild_search_artifacts(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    pending_document_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> Result<SearchPublicationDecision>,
) -> Result<SearchPublicationTransactionOutcome> {
    let store = publication_session.owned_store();
    let base = load_migration_rebuild_publication_base(store)?;
    publish_rebuilt_search_artifacts_from_base(
        publication_session,
        now,
        classifier_epoch,
        pending_document_ids,
        pending_index_documents,
        base,
        vectorization,
        ensure_not_cancelled,
        decide,
    )
}

pub(super) fn publish_rebuilt_search_artifacts_from_base(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
    classifier_epoch: &str,
    pending_document_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
    base: SearchPublicationBase,
    vectorization: &SearchPublicationVectorization,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> Result<SearchPublicationDecision>,
) -> Result<SearchPublicationTransactionOutcome> {
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    let data_dir = publication_session.canonical_data_dir();
    let store = publication_session.owned_store();
    let sectionizer = Sectionizer::default();
    let staged_version_texts =
        staged_search_version_texts(&pending_index_documents, ensure_not_cancelled)?;
    let mut index_documents = persisted_index_documents(
        store,
        &base.projections,
        &sectionizer,
        pending_document_ids,
        ensure_not_cancelled,
    )?;
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    index_documents.extend(pending_index_documents);
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    sort_index_documents(&mut index_documents);
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    let projections = index_documents
        .iter()
        .map(|document| {
            Ok(ActiveSearchProjection {
                document_id: document
                    .doc_id
                    .parse()
                    .map_err(|_| ImportPipelineError::store_invariant())?,
                resume_version_id: document
                    .resume_version_id
                    .parse()
                    .map_err(|_| ImportPipelineError::store_invariant())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let indexed_documents = index_documents.len();
    let generation = search_generation_token(now, indexed_documents, 0, 0);
    run_search_publication_transaction(
        publication_session,
        now,
        classifier_epoch,
        base,
        &generation,
        projections,
        &staged_version_texts,
        vectorization,
        ensure_not_cancelled,
        || {
            write_fulltext_snapshot(
                data_dir,
                &generation,
                index_documents,
                FullTextSnapshotInput::NeedsRedaction,
                ensure_not_cancelled,
                None,
                None,
                ImportResourcePolicy::detect().index_writer_heap_bytes,
            )
        },
        decide,
    )
}

fn write_fulltext_snapshot<I>(
    data_dir: &Path,
    generation: &str,
    index_documents: I,
    input: FullTextSnapshotInput,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<PublishedSnapshotMetadata>
where
    I: IntoIterator<Item = IndexDocument>,
{
    let index_root = data_dir.join("search-index");
    let cancel_check =
        || ensure_not_cancelled.is_some_and(|ensure_not_cancelled| ensure_not_cancelled().is_err());
    let phase_observer = |phase| {
        if let Some(set_cancel_phase) = set_cancel_phase {
            set_cancel_phase(ImportCancelCheckPhase::from_snapshot_publish_phase(phase));
        }
    };
    let mut control = if ensure_not_cancelled.is_some() {
        SnapshotPublishControl::from_cancel_check(&cancel_check)
    } else {
        SnapshotPublishControl::disabled()
    };
    if set_cancel_phase.is_some() {
        control = control.with_phase_observer(&phase_observer);
    }
    if let Some(record_phase_timing) = record_phase_timing {
        control = control.with_phase_timing_observer(record_phase_timing);
    }
    control = control.with_writer_heap_bytes(index_writer_heap_bytes);
    match input {
        FullTextSnapshotInput::NeedsRedaction => {
            publish_snapshot_with_control(&index_root, generation, index_documents, control)
        }
        FullTextSnapshotInput::AlreadyRedacted => publish_trusted_redacted_snapshot_with_control(
            &index_root,
            generation,
            index_documents,
            control,
        ),
    }
    .map_err(ImportPipelineError::index)
}

#[derive(Clone, Copy)]
enum FullTextSnapshotInput {
    NeedsRedaction,
    AlreadyRedacted,
}

fn write_cached_fulltext_snapshot(
    data_dir: &Path,
    generation: &str,
    documents: &[super::search_artifact_cache::CachedSearchDocument],
    sectionizer: &Sectionizer,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<PublishedSnapshotMetadata> {
    write_fulltext_snapshot(
        data_dir,
        generation,
        documents
            .iter()
            .map(|document| document.to_index_document(sectionizer)),
        FullTextSnapshotInput::AlreadyRedacted,
        ensure_not_cancelled,
        set_cancel_phase,
        record_phase_timing,
        index_writer_heap_bytes,
    )
}

fn write_cached_fulltext_snapshot_consuming(
    data_dir: &Path,
    generation: &str,
    documents: &mut Vec<super::search_artifact_cache::CachedSearchDocument>,
    sectionizer: &Sectionizer,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<PublishedSnapshotMetadata> {
    let documents = std::mem::take(documents);
    write_fulltext_snapshot(
        data_dir,
        generation,
        documents
            .into_iter()
            .map(|document| document.into_index_document(sectionizer)),
        FullTextSnapshotInput::AlreadyRedacted,
        ensure_not_cancelled,
        set_cancel_phase,
        record_phase_timing,
        index_writer_heap_bytes,
    )
}

fn persisted_index_documents(
    store: &OwnedMetaStore,
    projections: &[ActiveSearchProjection],
    sectionizer: &Sectionizer,
    pending_document_ids: &BTreeSet<String>,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
) -> Result<Vec<IndexDocument>> {
    let mut index_documents = Vec::with_capacity(projections.len());
    for projection in projections {
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
        if pending_document_ids.contains(projection.document_id.as_str()) {
            continue;
        }
        let Some(document) = store
            .active_search_document(projection)
            .map_err(ImportPipelineError::store)?
        else {
            return Err(ImportPipelineError::store_invariant());
        };
        let Some(version) = store
            .resume_version_by_id(&projection.resume_version_id)
            .map_err(ImportPipelineError::store)?
        else {
            return Err(ImportPipelineError::store_invariant());
        };
        if let Some(index_document) =
            index_document_from_resume_version(&document, &version, sectionizer)
        {
            index_documents.push(index_document);
        }
    }
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    Ok(index_documents)
}

pub(crate) fn index_document_from_resume_version(
    document: &Document,
    version: &ResumeVersion,
    sectionizer: &Sectionizer,
) -> Option<IndexDocument> {
    let clean_text = version.clean_text.as_ref()?;
    if clean_text.trim().is_empty() {
        return None;
    }
    Some(IndexDocument {
        doc_id: document.id.to_string(),
        resume_version_id: version.id.to_string(),
        file_name: document.file_name.clone(),
        clean_text: clean_text.clone(),
        sections: sections_to_index(sectionizer.sectionize(clean_text)),
    })
}

pub(super) fn migration_index_documents_from_exact_projection(
    projection_rows: Vec<MigrationRebuildProjectionRow>,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
) -> Result<Vec<(Document, IndexDocument)>> {
    let sectionizer = Sectionizer::default();
    let mut staged = Vec::with_capacity(projection_rows.len());
    for row in projection_rows {
        if let Some(check) = ensure_not_cancelled {
            check()?;
        }
        let index_document =
            index_document_from_resume_version(&row.document, &row.resume_version, &sectionizer)
                .ok_or_else(ImportPipelineError::store_invariant)?;
        staged.push((row.document, index_document));
    }
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    staged.sort_by(|left, right| {
        left.1
            .doc_id
            .cmp(&right.1.doc_id)
            .then_with(|| left.1.resume_version_id.cmp(&right.1.resume_version_id))
    });
    if let Some(check) = ensure_not_cancelled {
        check()?;
    }
    Ok(staged)
}

fn sort_index_documents(documents: &mut [IndexDocument]) {
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });
}

fn search_generation_token(
    now: UnixTimestamp,
    indexed_documents: usize,
    ocr_required_documents: usize,
    deleted_documents: usize,
) -> String {
    format!(
        "search-{}-{}-{indexed_documents}-{ocr_required_documents}-{deleted_documents}",
        now.as_unix_seconds(),
        generation_unique_suffix(now)
    )
}

fn generation_unique_suffix(now: UnixTimestamp) -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_else(|_| now.as_unix_seconds() as u128)
}
