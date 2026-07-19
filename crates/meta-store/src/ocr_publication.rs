use rusqlite::{params, Connection};

use crate::{
    assign_candidate_from_hashed_contacts_in_connection, classification, immutable_search,
    ingest_job_kind_to_storage, ingest_job_status_to_storage, u32_to_i64, ClaimedOcrJob,
    ClassificationStatus, ContactHash, CurrentClassifierEpoch, DocumentStatus, EntityMention,
    IngestJobKind, IngestJobStatus, MetaStoreError, Result, ResumeVersion,
    ResumeVersionClassification, SearchPublicationCommit, SourceRevision,
};

/// Immutable OCR-derived facts that may become durable only together with the
/// exact validated search publication and the exact running OCR claim.
pub struct OcrSearchPublicationCommit<'a> {
    pub search: SearchPublicationCommit<'a>,
    pub claimed: &'a ClaimedOcrJob,
    pub source_revision: &'a SourceRevision,
    pub version: &'a ResumeVersion,
    pub classification: &'a ResumeVersionClassification,
    pub mentions: &'a [EntityMention],
    pub email_hash: Option<&'a ContactHash>,
    pub phone_hash: Option<&'a ContactHash>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrSearchPublicationOutcome {
    Applied,
    ClaimSuperseded,
    PublicationSuperseded,
}

pub(crate) fn validate_ocr_search_publication_commit(
    publication: &OcrSearchPublicationCommit<'_>,
) -> Result<()> {
    let claimed = publication.claimed;
    let job = &claimed.job;
    let candidate = publication.classification.status == ClassificationStatus::ResumeCandidate;
    let terminal = publication.search.terminal_documents.first();
    let projected_version = publication
        .search
        .projections
        .iter()
        .find(|projection| projection.document_id == job.document_id)
        .map(|projection| &projection.resume_version_id);
    let valid_terminal = terminal.is_some_and(|terminal| {
        terminal.document_id == job.document_id
            && terminal.expected_status == DocumentStatus::OcrRequired
            && !terminal.expected_is_deleted
            && terminal.expected_content_hash.as_str() == claimed.source_fingerprint()
            && terminal.terminal_status
                == if candidate {
                    DocumentStatus::Searchable
                } else {
                    DocumentStatus::Excluded
                }
            && !terminal.terminal_is_deleted
    });
    let valid = job.kind == IngestJobKind::OcrDocument
        && job.status == IngestJobStatus::Running
        && job.attempt_count > 0
        && job.resume_version_id.is_none()
        && publication.search.terminal_documents.len() == 1
        && valid_terminal
        && publication.source_revision.document_id == job.document_id
        && publication.source_revision.id == *claimed.source_revision_id()
        && publication.source_revision.content_hash.as_str() == claimed.source_fingerprint()
        && publication.version.document_id == job.document_id
        && publication.version.source_revision_id == publication.source_revision.id
        && publication.classification.resume_version_id == publication.version.id
        && CurrentClassifierEpoch::parse(&publication.classification.classifier_epoch).is_some()
        && publication.classification.status != ClassificationStatus::OcrBacklog
        && (candidate == (projected_version == Some(&publication.version.id)))
        && (candidate || publication.mentions.is_empty())
        && (candidate || publication.email_hash.is_none() && publication.phone_hash.is_none());
    if valid {
        Ok(())
    } else {
        Err(MetaStoreError::invalid_value(
            "ingest_job.ocr_search_publication",
        ))
    }
}

pub(crate) fn insert_ocr_search_publication_facts_in_connection(
    connection: &Connection,
    publication: &OcrSearchPublicationCommit<'_>,
) -> Result<()> {
    immutable_search::insert_source_revision_in_connection(
        connection,
        publication.source_revision,
    )?;
    immutable_search::insert_resume_version_in_connection(connection, publication.version)?;
    classification::insert_resume_version_classification_in_connection(
        connection,
        publication.classification,
    )?;
    immutable_search::insert_entity_mentions_in_connection(
        connection,
        &publication.version.id,
        publication.mentions,
    )?;
    assign_candidate_from_hashed_contacts_in_connection(
        connection,
        &publication.version.id,
        publication.email_hash,
        publication.phone_hash,
        publication.search.now,
    )?;
    Ok(())
}

pub(crate) fn complete_ocr_search_publication_claim_in_connection(
    connection: &Connection,
    publication: &OcrSearchPublicationCommit<'_>,
) -> Result<()> {
    let job = &publication.claimed.job;
    let changed = connection
        .execute(
            "UPDATE ingest_job
             SET status = ?1, resume_version_id = ?2, finished_at_seconds = ?3,
                 updated_at_seconds = ?3, failure_kind = NULL
             WHERE id = ?4 AND document_id = ?5 AND kind = ?6 AND status = ?7
               AND attempt_count = ?8 AND max_attempts = ?9",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Completed),
                publication.version.id.as_str(),
                publication.search.now.as_unix_seconds(),
                job.id.as_str(),
                job.document_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                ingest_job_status_to_storage(IngestJobStatus::Running),
                u32_to_i64(job.attempt_count),
                u32_to_i64(job.max_attempts),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(MetaStoreError::invalid_transition())
    }
}
