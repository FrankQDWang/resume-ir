use fs_crawler::DiscoveredFile;
use meta_store::{
    ClassificationStatus, ContentDigest, CurrentClassifierEpoch, DocumentStatus, IngestJob,
    IngestJobStatus, OwnedMetaStore, SourceRevision, UnixTimestamp,
};
use resume_classifier::LinearPromotionPolicy;
use sectionizer::Sectionizer;

use super::model::{
    ExactRerunDecision, PendingSearchableDocument, PendingSearchablePublicationKind,
};
use crate::search_artifacts::index_document_from_resume_version;
use crate::{ImportPipelineError, Result, OCR_PARSE_VERSION, PARSE_VERSION, SCHEMA_VERSION};

pub(crate) fn exact_rerun_decision(
    store: &OwnedMetaStore,
    file: &DiscoveredFile,
    strong_content_hash: &ContentDigest,
    linear_promotion: &LinearPromotionPolicy,
    now: UnixTimestamp,
) -> Result<Option<ExactRerunDecision>> {
    let Some(mut document) = store
        .document_by_id(&file.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(None);
    };

    if document.is_deleted
        || document.extension != file.extension
        || document.byte_size != file.byte_size
        || document.content_hash.as_deref() != Some(strong_content_hash.as_str())
    {
        return Ok(None);
    }

    let source_revision = SourceRevision::for_content(
        document.id.clone(),
        strong_content_hash.clone(),
        file.byte_size,
    );
    let classifier_epoch = linear_promotion
        .classifier_epoch()
        .unwrap_or(meta_store::CLASSIFIER_EPOCH);

    match document.status {
        DocumentStatus::Searchable | DocumentStatus::IndexedPartial => {
            let Some(active_projection) = store
                .active_search_projection_for_document(&document.id)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            let Some(version) = store
                .resume_version_by_id(&active_projection.resume_version_id)
                .map_err(ImportPipelineError::store)?
            else {
                return Err(ImportPipelineError::store_invariant());
            };
            if version.source_revision_id != source_revision.id
                || version.schema_version != SCHEMA_VERSION
                || !matches!(
                    version.parse_version.as_str(),
                    PARSE_VERSION | OCR_PARSE_VERSION
                )
            {
                return Ok(None);
            }
            let Some(classification) = store
                .resume_version_classification(&version.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            if !classification_epoch_matches(classifier_epoch, &classification.classifier_epoch)
                || classification.status != ClassificationStatus::ResumeCandidate
            {
                return Ok(None);
            }
            let active_document = store
                .active_search_document(&active_projection)
                .map_err(ImportPipelineError::store)?
                .ok_or_else(ImportPipelineError::store_invariant)?;
            let Some(projected_document) = changed_projected_document(&active_document, file, now)
            else {
                return Ok(Some(ExactRerunDecision::UnchangedSearchable {
                    source_revision_id: source_revision.id,
                    resume_version_id: version.id,
                }));
            };
            store
                .upsert_document(&projected_document)
                .map_err(ImportPipelineError::store)?;
            let mentions = store
                .entity_mentions_for_version(&version.id)
                .map_err(ImportPipelineError::store)?;
            let index_document = index_document_from_resume_version(
                &projected_document,
                &version,
                &Sectionizer::default(),
            )
            .ok_or_else(ImportPipelineError::store_invariant)?;
            Ok(Some(ExactRerunDecision::MetadataChangedSearchable {
                pending: Box::new(PendingSearchableDocument {
                    document: projected_document,
                    source_revision,
                    classification,
                    version,
                    mentions,
                    email_hash: None,
                    phone_hash: None,
                    index_document,
                    publication_kind: PendingSearchablePublicationKind::MetadataChanged,
                }),
            }))
        }
        DocumentStatus::OcrRequired => {
            update_nonprojected_document_metadata(store, &mut document, file, now)?;
            let Some(triage) = store
                .source_revision_triage(&source_revision.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            if !classification_epoch_matches(classifier_epoch, &triage.triage_epoch)
                || triage.status != ClassificationStatus::OcrBacklog
            {
                return Ok(None);
            }
            let triage_epoch = CurrentClassifierEpoch::parse(classifier_epoch)
                .ok_or_else(ImportPipelineError::store_invariant)?;
            let job = store
                .ocr_job_for_source_triage(&source_revision.id, triage_epoch)
                .map_err(ImportPipelineError::store)?;
            Ok(job.as_ref().is_some_and(ocr_job_is_actionable).then_some(
                ExactRerunDecision::UnchangedOcrRequired {
                    source_revision_id: source_revision.id,
                },
            ))
        }
        DocumentStatus::Excluded => {
            update_nonprojected_document_metadata(store, &mut document, file, now)?;
            let mut matching = store
                .resume_versions_for_document(&document.id)
                .map_err(ImportPipelineError::store)?
                .into_iter()
                .filter(|version| {
                    version.source_revision_id == source_revision.id
                        && matches!(
                            version.parse_version.as_str(),
                            PARSE_VERSION | OCR_PARSE_VERSION
                        )
                        && version.schema_version == SCHEMA_VERSION
                });
            let Some(version) = matching.next() else {
                return Ok(None);
            };
            if matching.next().is_some() {
                return Err(ImportPipelineError::store_invariant());
            }
            let Some(classification) = store
                .resume_version_classification(&version.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            if !classification_epoch_matches(classifier_epoch, &classification.classifier_epoch)
                || !matches!(
                    classification.status,
                    ClassificationStatus::NonResume | ClassificationStatus::NeedsReview
                )
            {
                return Ok(None);
            }
            Ok(Some(ExactRerunDecision::UnchangedExcluded {
                source_revision_id: source_revision.id,
                resume_version_id: version.id,
            }))
        }
        _ => Ok(None),
    }
}

fn changed_projected_document(
    active: &meta_store::Document,
    file: &DiscoveredFile,
    now: UnixTimestamp,
) -> Option<meta_store::Document> {
    let source_uri = format!("file://{}", file.normalized_path.as_str());
    if active.source_uri == source_uri
        && active.normalized_path == file.normalized_path.as_str()
        && active.file_name == file.file_name
        && active.mtime == file.mtime
    {
        return None;
    }
    let mut changed = active.clone();
    changed.source_uri = source_uri;
    changed.normalized_path = file.normalized_path.as_str().to_string();
    changed.file_name = file.file_name.clone();
    changed.mtime = file.mtime;
    changed.updated_at = now;
    Some(changed)
}

fn update_nonprojected_document_metadata(
    store: &OwnedMetaStore,
    document: &mut meta_store::Document,
    file: &DiscoveredFile,
    now: UnixTimestamp,
) -> Result<()> {
    let source_uri = format!("file://{}", file.normalized_path.as_str());
    if document.source_uri == source_uri
        && document.normalized_path == file.normalized_path.as_str()
        && document.file_name == file.file_name
        && document.mtime == file.mtime
    {
        return Ok(());
    }
    document.source_uri = source_uri;
    document.normalized_path = file.normalized_path.as_str().to_string();
    document.file_name = file.file_name.clone();
    document.mtime = file.mtime;
    document.updated_at = now;
    store
        .upsert_document(document)
        .map_err(ImportPipelineError::store)
}

pub(crate) fn classification_epoch_matches(expected: &str, actual: &str) -> bool {
    CurrentClassifierEpoch::parse(actual).is_some_and(|epoch| epoch.as_str() == expected)
}

pub(crate) fn ocr_job_is_actionable(job: &IngestJob) -> bool {
    match job.status {
        IngestJobStatus::Queued | IngestJobStatus::Running => true,
        IngestJobStatus::Interrupted | IngestJobStatus::FailedRetryable => {
            job.attempt_count < job.max_attempts
        }
        IngestJobStatus::Completed | IngestJobStatus::FailedPermanent => false,
    }
}
