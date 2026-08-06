use fs_crawler::DiscoveredFile;
use meta_store::{
    ImmutableIngestStage, ImportTaskId, ImportTaskPurpose, OwnedMetaStore,
    SourceRootBoundImportCommit, SourceRootBoundImportCommitOutcome, SourceRootBoundImportMutation,
    SourceRootBoundImportObservation, UnixTimestamp,
};

use super::{
    PendingExistingDocument, PendingSearchableCommitRoute, PendingSearchablePublicationKind,
};
use crate::file_observation_fast_path::strong_store_observation;
use crate::source_dispositions::ProcessedFile;
use crate::verified_content::ContentVerification;
use crate::{ImportPipelineError, Result};

pub(crate) fn commit_import_file(
    store: &OwnedMetaStore,
    purpose: ImportTaskPurpose,
    task_id: &ImportTaskId,
    file: &DiscoveredFile,
    processed: &mut ProcessedFile,
    verification: ContentVerification,
    now: UnixTimestamp,
) -> Result<SourceRootBoundImportCommitOutcome> {
    match purpose {
        ImportTaskPurpose::ConfiguredCatchUp => {
            commit_configured_import(store, task_id, file, processed, verification, now)
        }
        ImportTaskPurpose::MigrationRebuildFullCorpus => {
            commit_migration_rebuild(store, task_id, file, processed, verification, now)
        }
    }
}

fn commit_configured_import(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    file: &DiscoveredFile,
    processed: &mut ProcessedFile,
    verification: ContentVerification,
    now: UnixTimestamp,
) -> Result<SourceRootBoundImportCommitOutcome> {
    let strong_observation = strong_observation(file, processed, verification, now)?;
    let observation = strong_observation.as_ref().map_or(
        SourceRootBoundImportObservation::MetadataOnly,
        SourceRootBoundImportObservation::Strong,
    );
    let outcome = store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id,
            normalized_path: file.normalized_path.as_str(),
            observed_at: now,
            mutation: mutation(processed, &file.document_id),
            observation,
        })
        .map_err(ImportPipelineError::store)?;
    if let ProcessedFile::Searchable { pending } = processed {
        pending.commit_route = PendingSearchableCommitRoute::RootBoundCommitted;
    }
    Ok(outcome)
}

fn mutation<'a>(
    processed: &'a ProcessedFile,
    document_id: &'a meta_store::DocumentId,
) -> SourceRootBoundImportMutation<'a> {
    match processed {
        ProcessedFile::Searchable { pending } => match pending.publication_kind {
            PendingSearchablePublicationKind::Replacement => {
                SourceRootBoundImportMutation::Immutable(ImmutableIngestStage::ClassifiedResume {
                    document: &pending.document,
                    source_revision: &pending.source_revision,
                    version: &pending.version,
                    classification: &pending.classification,
                    mentions: &pending.mentions,
                    email_hash: pending.email_hash.as_ref(),
                    phone_hash: pending.phone_hash.as_ref(),
                })
            }
            PendingSearchablePublicationKind::MetadataChanged => {
                SourceRootBoundImportMutation::ExistingRevisionMetadata {
                    document: &pending.document,
                    source_revision_id: &pending.source_revision.id,
                }
            }
        },
        ProcessedFile::UnchangedSearchable {
            source_revision_id, ..
        } => SourceRootBoundImportMutation::ExistingRevision {
            document_id,
            source_revision_id,
        },
        ProcessedFile::UnchangedOcrRequired {
            document,
            source_revision_id,
        }
        | ProcessedFile::UnchangedExcluded {
            document,
            source_revision_id,
            ..
        } => existing_mutation(document, source_revision_id),
        ProcessedFile::Excluded { pending } => {
            SourceRootBoundImportMutation::Immutable(ImmutableIngestStage::ClassifiedResume {
                document: &pending.document,
                source_revision: &pending.source_revision,
                version: &pending.version,
                classification: &pending.classification,
                mentions: &[],
                email_hash: None,
                phone_hash: None,
            })
        }
        ProcessedFile::OcrRequired { pending } => {
            SourceRootBoundImportMutation::OcrRequired(ImmutableIngestStage::SourceTriage {
                document: &pending.document,
                source_revision: &pending.source_revision,
                triage: &pending.triage,
            })
        }
        ProcessedFile::Failed {
            pending: Some(pending),
            ..
        } => SourceRootBoundImportMutation::Immutable(ImmutableIngestStage::SourceTriage {
            document: &pending.document,
            source_revision: &pending.source_revision,
            triage: &pending.triage,
        }),
        ProcessedFile::Failed { pending: None, .. } => {
            SourceRootBoundImportMutation::ReadFailureWithoutRevision
        }
    }
}

fn existing_mutation<'a>(
    document: &'a PendingExistingDocument,
    source_revision_id: &'a meta_store::SourceRevisionId,
) -> SourceRootBoundImportMutation<'a> {
    match document {
        PendingExistingDocument::Unchanged(document) => {
            SourceRootBoundImportMutation::ExistingRevision {
                document_id: &document.id,
                source_revision_id,
            }
        }
        PendingExistingDocument::MetadataChanged(document) => {
            SourceRootBoundImportMutation::ExistingRevisionMetadata {
                document,
                source_revision_id,
            }
        }
    }
}

fn strong_observation(
    file: &DiscoveredFile,
    processed: &ProcessedFile,
    verification: ContentVerification,
    now: UnixTimestamp,
) -> Result<Option<meta_store::StrongSourceFileObservation>> {
    if verification != ContentVerification::Strong {
        return Ok(None);
    }
    let observation = file
        .observation
        .as_ref()
        .ok_or_else(ImportPipelineError::source_changed_during_import)?;
    let source_revision_id =
        source_revision_id(processed).ok_or_else(ImportPipelineError::store_invariant)?;
    Ok(Some(strong_store_observation(
        source_revision_id.clone(),
        observation,
        now,
    )))
}

fn source_revision_id(processed: &ProcessedFile) -> Option<&meta_store::SourceRevisionId> {
    match processed {
        ProcessedFile::Searchable { pending } => Some(&pending.source_revision.id),
        ProcessedFile::UnchangedSearchable {
            source_revision_id, ..
        }
        | ProcessedFile::UnchangedOcrRequired {
            source_revision_id, ..
        }
        | ProcessedFile::UnchangedExcluded {
            source_revision_id, ..
        } => Some(source_revision_id),
        ProcessedFile::Excluded { pending } => Some(&pending.source_revision.id),
        ProcessedFile::OcrRequired { pending }
        | ProcessedFile::Failed {
            pending: Some(pending),
            ..
        } => Some(&pending.source_revision.id),
        ProcessedFile::Failed { pending: None, .. } => None,
    }
}

fn commit_migration_rebuild(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    file: &DiscoveredFile,
    processed: &mut ProcessedFile,
    verification: ContentVerification,
    now: UnixTimestamp,
) -> Result<SourceRootBoundImportCommitOutcome> {
    if let ProcessedFile::Searchable { pending } = processed {
        pending.commit_route = PendingSearchableCommitRoute::MigrationRebuildPrepared;
        return Ok(SourceRootBoundImportCommitOutcome::default());
    }
    let mut outcome = SourceRootBoundImportCommitOutcome::default();
    match mutation(processed, &file.document_id) {
        SourceRootBoundImportMutation::Immutable(stage) => store
            .stage_immutable_ingest(stage)
            .map_err(ImportPipelineError::store)?,
        SourceRootBoundImportMutation::OcrRequired(stage) => {
            let (source_revision_id, triage_epoch) = match &stage {
                ImmutableIngestStage::SourceTriage {
                    source_revision,
                    triage,
                    ..
                } => (source_revision.id.clone(), triage.triage_epoch.clone()),
                ImmutableIngestStage::ClassifiedResume { .. } => {
                    return Err(ImportPipelineError::store_invariant());
                }
            };
            store
                .stage_immutable_ingest(stage)
                .map_err(ImportPipelineError::store)?;
            let triage_epoch = meta_store::SourceTriageEpoch::parse(&triage_epoch)
                .map_err(ImportPipelineError::store)?;
            outcome.ocr_job_scheduled = store
                .enqueue_ocr_job_for_source_triage(&source_revision_id, &triage_epoch, now)
                .map_err(ImportPipelineError::store)?
                .scheduled;
        }
        SourceRootBoundImportMutation::ExistingRevision { .. } => {}
        SourceRootBoundImportMutation::ExistingRevisionMetadata { document, .. } => store
            .upsert_document(document)
            .map_err(ImportPipelineError::store)?,
        SourceRootBoundImportMutation::ReadFailureWithoutRevision => return Ok(outcome),
    }
    let source_revision_id =
        source_revision_id(processed).ok_or_else(ImportPipelineError::store_invariant)?;
    store
        .observe_import_task_source_occurrence(
            task_id,
            file.normalized_path.as_str(),
            &file.document_id,
            source_revision_id,
            now,
        )
        .map_err(ImportPipelineError::store)?;
    if let Some(observation) = strong_observation(file, processed, verification, now)? {
        store
            .record_strong_source_file_observation(
                task_id,
                file.normalized_path.as_str(),
                &observation,
            )
            .map_err(ImportPipelineError::store)?;
    }
    Ok(outcome)
}
