use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use index_fulltext::IndexDocument;
use meta_store::{
    Document, DocumentId, DocumentStatus, OwnedMetaStore, SearchProjectionServiceState,
    SearchPublicationSession, SearchRepairReason, UnixTimestamp,
};

use crate::immutable_ingest::{self, StagedDerivedData, StagedResume};
use crate::search_artifact_cache::{CurrentImportCacheMode, CurrentImportDocumentCache};
use crate::search_artifacts::{
    publish_incremental_search_artifacts, publish_rebuilt_search_artifacts,
};
use crate::search_publication_commit::{
    decide_search_publication, decide_search_publication_cancellable,
};
use crate::{
    measure_result_stage, ImportCancelCheckPhase, ImportMilestoneTimings, ImportPipelineError,
    ImportResourcePolicy, ImportSummary, ImportWorkerMetrics, PendingSearchableDocument,
    PendingSearchablePublicationKind, Result, SearchArtifactPublicationSummary,
    SearchProjectionRemoval, SearchProjectionRemovalReason, SearchPublicationVectorization,
};

struct ScheduledProjectionRemoval {
    reason: SearchProjectionRemovalReason,
    document_update: Option<Document>,
}

#[derive(Default)]
pub(super) struct PendingProjectionRemovals(BTreeMap<DocumentId, ScheduledProjectionRemoval>);

impl PendingProjectionRemovals {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn schedule(
        &mut self,
        document_id: DocumentId,
        reason: SearchProjectionRemovalReason,
        document_update: Option<Document>,
    ) -> Result<()> {
        if let Some(existing) = self.0.get_mut(&document_id) {
            if existing.reason != reason {
                return Err(ImportPipelineError::store_invariant());
            }
            match (&existing.document_update, document_update) {
                (Some(existing), Some(replacement)) if existing != &replacement => {
                    return Err(ImportPipelineError::store_invariant());
                }
                (None, Some(replacement)) => existing.document_update = Some(replacement),
                _ => {}
            }
            return Ok(());
        }
        self.0.insert(
            document_id,
            ScheduledProjectionRemoval {
                reason,
                document_update,
            },
        );
        Ok(())
    }

    fn document_ids(&self) -> BTreeSet<String> {
        self.0
            .keys()
            .map(|document_id| document_id.as_str().to_string())
            .collect()
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn publication_documents(&self) -> impl Iterator<Item = &Document> {
        self.0
            .values()
            .filter_map(|removal| removal.document_update.as_ref())
    }

    fn retain_active_projections(
        &mut self,
        publication_session: &SearchPublicationSession,
    ) -> Result<()> {
        let store = publication_session.owned_store();
        let mut retained = BTreeMap::new();
        for (document_id, removal) in std::mem::take(&mut self.0) {
            if store
                .active_search_projection_for_document(&document_id)
                .map_err(ImportPipelineError::store)?
                .is_some()
            {
                retained.insert(document_id, removal);
            }
        }
        self.0 = retained;
        Ok(())
    }
}

/// Publishes all currently staged searchable replacements and removals through
/// one owner-bound session. Staging never becomes query-visible before the
/// artifact validation and metadata CAS both succeed.
pub(super) fn flush_pending_searchable_documents(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    current_import_index_documents: Option<&mut CurrentImportDocumentCache>,
    current_import_index_cache_mode: CurrentImportCacheMode,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
) -> Result<bool> {
    if !pending_index_documents.is_empty() {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        ensure_not_cancelled()?;
        measure_result_stage(&mut summary.stage_timings.db, || {
            for pending in pending_index_documents.iter() {
                match pending.publication_kind {
                    PendingSearchablePublicationKind::Replacement => {
                        immutable_ingest::stage(
                            store,
                            StagedResume {
                                document: &pending.document,
                                source_revision: &pending.source_revision,
                                derived: StagedDerivedData::ClassifiedVersion {
                                    version: &pending.version,
                                    classification: &pending.classification,
                                    mentions: &pending.mentions,
                                    email_hash: pending.email_hash.as_ref(),
                                    phone_hash: pending.phone_hash.as_ref(),
                                },
                            },
                        )
                        .map_err(ImportPipelineError::store)?;
                    }
                    PendingSearchablePublicationKind::MetadataChanged => {
                        // The immutable version and derived facts are already active. The
                        // exact mutable Document snapshot was staged by rerun detection and
                        // is published only by the generation CAS below.
                    }
                }
                if let Some(occurrence) = &pending.source_occurrence {
                    store
                        .observe_import_task_source_occurrence(
                            &occurrence.task_id,
                            &occurrence.normalized_path,
                            &pending.document.id,
                            &pending.source_revision.id,
                            occurrence.observed_at,
                        )
                        .map_err(ImportPipelineError::store)?;
                }
            }
            Ok(())
        })?;
    }

    set_cancel_phase(ImportCancelCheckPhase::IndexPublication);
    ensure_not_cancelled()?;
    let publication_session = store
        .wait_for_search_publication_session()
        .map_err(ImportPipelineError::store)?;
    let publication_store = publication_session.owned_store();
    let projection_state = publication_store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?;
    let needs_initial_publication = projection_state.generation.is_none();
    let migration_rebuild_staging = projection_state.generation.is_none()
        && projection_state.service_state == SearchProjectionServiceState::Repairing
        && projection_state.repair_reason == Some(SearchRepairReason::MigrationRebuild);
    let classifier_epoch =
        publication_classifier_epoch(publication_store, pending_index_documents)?;

    if migration_rebuild_staging {
        if pending_index_documents.iter().any(|pending| {
            pending.publication_kind != PendingSearchablePublicationKind::Replacement
        }) {
            return Err(ImportPipelineError::store_invariant());
        }
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        for document in pending_excluded_doc_ids.publication_documents() {
            ensure_not_cancelled()?;
            publication_store
                .upsert_document(document)
                .map_err(ImportPipelineError::store)?;
        }
        summary.searchable_documents += pending_index_documents.len();
        pending_index_documents.clear();
        pending_excluded_doc_ids.clear();
        return Ok(false);
    }

    let unchanged_searchable_documents =
        normalize_searchable_replacements(&publication_session, pending_index_documents)?;
    pending_excluded_doc_ids.retain_active_projections(&publication_session)?;
    let has_delta = !pending_index_documents.is_empty() || !pending_excluded_doc_ids.is_empty();
    if !has_delta && !needs_initial_publication {
        summary.searchable_documents += unchanged_searchable_documents;
        return Ok(false);
    }

    let removed_document_ids = pending_excluded_doc_ids.document_ids();
    let searchable_before = summary.searchable_documents;
    let (mut pending_documents, pending_replacements) =
        take_pending_searchable_documents(pending_index_documents);
    let phase_worker_metrics = RefCell::new(ImportWorkerMetrics::default());
    let record_phase_timing = |phase, elapsed| {
        phase_worker_metrics
            .borrow_mut()
            .record_index_publication_phase_timing(phase, elapsed);
    };
    let index_started = Instant::now();
    for document in &mut pending_documents {
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
    }
    let new_searchable_count = pending_documents.len();
    pending_documents.extend(pending_excluded_doc_ids.publication_documents().cloned());

    let index_elapsed = RefCell::new(None);
    let write_result = publish_incremental_search_artifacts(
        &publication_session,
        now,
        &classifier_epoch,
        pending_replacements,
        &removed_document_ids,
        summary.ocr_required_documents,
        summary.deleted_documents,
        current_import_index_documents,
        current_import_index_cache_mode,
        Some(ensure_not_cancelled),
        Some(set_cancel_phase),
        Some(&record_phase_timing),
        index_writer_heap_bytes,
        search_vectorization,
        |publication| {
            *index_elapsed.borrow_mut() = Some(index_started.elapsed());
            set_cancel_phase(ImportCancelCheckPhase::DbWrite);
            measure_result_stage(&mut summary.stage_timings.db, || {
                decide_search_publication_cancellable(
                    publication,
                    now,
                    &pending_documents,
                    ensure_not_cancelled,
                )
            })
        },
    );
    summary.stage_timings.index += index_elapsed
        .into_inner()
        .unwrap_or_else(|| index_started.elapsed());
    summary
        .worker_metrics
        .add_assign(&phase_worker_metrics.into_inner());
    let committed_publication = write_result?.into_committed()?;
    committed_publication.release();
    summary.searchable_documents += unchanged_searchable_documents + new_searchable_count;
    pending_excluded_doc_ids.clear();
    let index_ready_elapsed = import_started.elapsed();
    record_searchable_milestones(
        &mut summary.milestone_timings,
        searchable_before,
        summary.searchable_documents,
        index_ready_elapsed,
    );
    Ok(true)
}

fn normalize_searchable_replacements(
    publication_session: &SearchPublicationSession,
    pending_documents: &mut Vec<PendingSearchableDocument>,
) -> Result<usize> {
    let mut retained = Vec::with_capacity(pending_documents.len());
    let mut unchanged = 0;
    for pending in std::mem::take(pending_documents) {
        let is_semantic_delta = match pending.publication_kind {
            PendingSearchablePublicationKind::MetadataChanged => true,
            PendingSearchablePublicationKind::Replacement => {
                searchable_replacement_changes_projection(publication_session, &pending)?
            }
        };
        if is_semantic_delta {
            retained.push(pending);
        } else {
            unchanged += 1;
        }
    }
    *pending_documents = retained;
    Ok(unchanged)
}

fn searchable_replacement_changes_projection(
    publication_session: &SearchPublicationSession,
    pending: &PendingSearchableDocument,
) -> Result<bool> {
    let store = publication_session.owned_store();
    let Some(active_projection) = store
        .active_search_projection_for_document(&pending.document.id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(true);
    };
    if active_projection.resume_version_id != pending.version.id {
        return Ok(true);
    }
    let active_document = store
        .active_search_document(&active_projection)
        .map_err(ImportPipelineError::store)?
        .ok_or_else(ImportPipelineError::store_invariant)?;
    let mut projected_document = pending.document.clone();
    projected_document.status = DocumentStatus::Searchable;
    projected_document.updated_at = active_document.updated_at;
    Ok(projected_document != active_document)
}

fn publication_classifier_epoch(
    store: &OwnedMetaStore,
    pending: &[PendingSearchableDocument],
) -> Result<String> {
    let pending_epochs = pending
        .iter()
        .map(|document| document.classification.classifier_epoch.as_str())
        .collect::<BTreeSet<_>>();
    if pending_epochs.len() > 1 {
        return Err(ImportPipelineError::store_invariant());
    }
    let pending_epoch = pending_epochs.first().copied();
    let current_epoch = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?
        .publication
        .map(|publication| publication.classifier_epoch.clone());
    if let (Some(pending_epoch), Some(current_epoch)) = (pending_epoch, current_epoch.as_deref()) {
        if pending_epoch != current_epoch {
            return Err(ImportPipelineError::store_invariant());
        }
    }
    Ok(pending_epoch
        .map(str::to_string)
        .or(current_epoch)
        .unwrap_or_else(|| resume_classifier::CLASSIFIER_EPOCH.to_string()))
}

pub(super) fn take_pending_searchable_documents(
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
) -> (Vec<Document>, Vec<IndexDocument>) {
    let pending = std::mem::take(pending_index_documents);
    let mut documents = Vec::with_capacity(pending.len());
    let mut index_documents = Vec::with_capacity(pending.len());
    for pending in pending {
        documents.push(pending.document);
        index_documents.push(pending.index_document);
    }
    (documents, index_documents)
}

fn record_searchable_milestones(
    milestones: &mut ImportMilestoneTimings,
    searchable_before: usize,
    searchable_after: usize,
    elapsed: Duration,
) {
    milestones.full_index_ready = Some(elapsed);
    if searchable_before == searchable_after {
        return;
    }
    if milestones.first_searchable.is_none() && searchable_after > 0 {
        milestones.first_searchable = Some(elapsed);
    }
    if milestones.ttf100_searchable.is_none() && searchable_before < 100 && searchable_after >= 100
    {
        milestones.ttf100_searchable = Some(elapsed);
    }
    if milestones.ttf1000_searchable.is_none()
        && searchable_before < 1000
        && searchable_after >= 1000
    {
        milestones.ttf1000_searchable = Some(elapsed);
    }
}

pub fn rebuild_search_artifacts(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactPublicationSummary> {
    let publication_session = store
        .wait_for_search_publication_session()
        .map_err(ImportPipelineError::store)?;
    let classifier_epoch = publication_classifier_epoch(store, &[])?;
    let publication = publish_rebuilt_search_artifacts(
        &publication_session,
        now,
        &classifier_epoch,
        &BTreeSet::new(),
        Vec::new(),
        vectorization,
        |publication| decide_search_publication(publication, now, &[]),
    )?
    .into_committed()?
    .release();

    Ok(SearchArtifactPublicationSummary {
        active_projection_count: publication.projections.len(),
    })
}

pub fn publish_search_projection_removals(
    store: &OwnedMetaStore,
    removals: &[SearchProjectionRemoval],
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactPublicationSummary> {
    let mut documents = Vec::with_capacity(removals.len());
    for removal in removals {
        let Some(mut document) = store
            .document_by_id(&removal.document_id)
            .map_err(ImportPipelineError::store)?
        else {
            continue;
        };
        if matches!(
            removal.reason,
            SearchProjectionRemovalReason::ConfirmedSourceDeletion
                | SearchProjectionRemovalReason::PrivacyRevocation
        ) {
            document.is_deleted = true;
            document.status = DocumentStatus::Deleted;
        }
        document.updated_at = now;
        documents.push(document);
    }
    let document_ids = removals
        .iter()
        .map(|removal| removal.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let publication_session = store
        .wait_for_search_publication_session()
        .map_err(ImportPipelineError::store)?;
    let publication = publish_incremental_search_artifacts(
        &publication_session,
        now,
        &publication_classifier_epoch(store, &[])?,
        Vec::new(),
        &document_ids,
        0,
        removals.len(),
        None,
        CurrentImportCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
        vectorization,
        |publication| decide_search_publication(publication, now, &documents),
    )?
    .into_committed()?
    .release();

    Ok(SearchArtifactPublicationSummary {
        active_projection_count: publication.projections.len(),
    })
}
