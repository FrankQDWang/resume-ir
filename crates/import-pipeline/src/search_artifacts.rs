use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use index_fulltext::{
    incremental_snapshot_documents, publish_snapshot_with_control,
    publish_trusted_redacted_snapshot_with_control, IndexDocument, PublishedSnapshotMetadata,
    SnapshotPublishControl, SnapshotPublishPhase,
};
use meta_store::{ActiveSearchProjection, Document, MetaStore, ResumeVersion, UnixTimestamp};
use sectionizer::Sectionizer;

use super::index_publication::SearchPublicationLock;
use super::search_artifact_cache::{
    apply_document_delta, ensure_cache_matches_generation, CurrentImportCacheMode,
    CurrentImportDocumentCache,
};
use super::search_publication::{
    load_search_publication_base, prepare_search_publication, projections_after_delta,
    PreparedSearchPublication, SearchPublicationBase,
};
use super::{
    sections_to_index, ImportCancelCheckPhase, ImportPipelineError, ImportResourcePolicy, Result,
    SearchPublicationVectorization,
};

pub(super) fn write_incremental_search_artifacts(
    data_dir: &Path,
    store: &MetaStore,
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
) -> Result<PreparedSearchPublication> {
    let publication_lock =
        SearchPublicationLock::acquire(data_dir).map_err(|_| ImportPipelineError::index_io())?;
    let base = load_search_publication_base(store)?;
    let index_root = data_dir.join("search-index");
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
        let publication = prepare_search_publication(
            data_dir,
            store,
            now,
            classifier_epoch,
            publication_lock,
            base,
            &generation,
            next_projections,
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
        current_import_cache.base_generation = Some(publication.fulltext.generation().to_string());
        if publication.fulltext.document_count() != indexed_documents {
            return Err(ImportPipelineError::store_invariant());
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
    let publication = prepare_search_publication(
        data_dir,
        store,
        now,
        classifier_epoch,
        publication_lock,
        base,
        &generation,
        next_projections,
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
    )?;
    if publication.fulltext.document_count() != indexed_documents {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(publication)
}

pub(super) fn write_rebuilt_search_artifacts(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    classifier_epoch: &str,
    publication_lock: SearchPublicationLock,
    pending_document_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
    vectorization: &SearchPublicationVectorization,
) -> Result<PreparedSearchPublication> {
    let base = load_search_publication_base(store)?;
    write_rebuilt_search_artifacts_from_base(
        data_dir,
        store,
        now,
        classifier_epoch,
        publication_lock,
        pending_document_ids,
        pending_index_documents,
        base,
        vectorization,
    )
}

pub(super) fn write_rebuilt_search_artifacts_from_base(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    classifier_epoch: &str,
    publication_lock: SearchPublicationLock,
    pending_document_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
    base: SearchPublicationBase,
    vectorization: &SearchPublicationVectorization,
) -> Result<PreparedSearchPublication> {
    let sectionizer = Sectionizer::default();
    let mut index_documents =
        persisted_index_documents(store, &base.projections, &sectionizer, pending_document_ids)?;
    index_documents.extend(pending_index_documents);
    sort_index_documents(&mut index_documents);
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
    let publication = prepare_search_publication(
        data_dir,
        store,
        now,
        classifier_epoch,
        publication_lock,
        base,
        &generation,
        projections,
        vectorization,
        None,
        || {
            write_fulltext_snapshot(
                data_dir,
                &generation,
                index_documents,
                FullTextSnapshotInput::NeedsRedaction,
                None,
                None,
                None,
                ImportResourcePolicy::detect().index_writer_heap_bytes,
            )
        },
    )?;
    if publication.fulltext.document_count() != indexed_documents {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(publication)
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
    store: &MetaStore,
    projections: &[ActiveSearchProjection],
    sectionizer: &Sectionizer,
    pending_document_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>> {
    let mut index_documents = Vec::with_capacity(projections.len());
    for projection in projections {
        if pending_document_ids.contains(projection.document_id.as_str()) {
            continue;
        }
        let Some(document) = store
            .document_by_id(&projection.document_id)
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
    Ok(index_documents)
}

fn index_document_from_resume_version(
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
