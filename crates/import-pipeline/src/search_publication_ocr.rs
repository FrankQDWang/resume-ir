use meta_store::{
    ClaimedOcrJob, ContactHash, Document, DocumentStatus, EntityMention,
    OcrSearchPublicationCommit, OcrSearchPublicationOutcome, ResumeVersion,
    ResumeVersionClassification, SearchPublicationCommit, SourceRevision, TerminalDocumentUpdate,
    UnixTimestamp,
};

use super::search_publication::{SearchPublicationDecision, SearchPublicationView};
use super::search_publication_commit::{
    bind_projected_document_snapshots, publication_document_map,
};
use super::{ImportPipelineError, Result};

pub(super) struct OcrPublicationFacts<'a> {
    pub(super) document: &'a Document,
    pub(super) claimed: &'a ClaimedOcrJob,
    pub(super) source_revision: &'a SourceRevision,
    pub(super) version: &'a ResumeVersion,
    pub(super) classification: &'a ResumeVersionClassification,
    pub(super) mentions: &'a [EntityMention],
    pub(super) email_hash: Option<&'a ContactHash>,
    pub(super) phone_hash: Option<&'a ContactHash>,
}

pub(super) enum OcrPublicationDecisionOutcome {
    ClaimSuperseded,
    PublicationSuperseded,
}

pub(super) fn decide_ocr_search_publication(
    now: UnixTimestamp,
    publication: &SearchPublicationView<'_>,
    facts: OcrPublicationFacts<'_>,
) -> Result<(
    SearchPublicationDecision,
    Option<OcrPublicationDecisionOutcome>,
)> {
    let fulltext = publication.fulltext();
    let vector = publication.vector();
    let projections = publication.projections();
    if fulltext.document_count() != projections.len()
        || vector.projection_count() != projections.len()
        || fulltext.generation() != vector.generation()
        || fulltext.generation().is_empty()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    let terminal_documents = [TerminalDocumentUpdate {
        document_id: facts.claimed.job.document_id.clone(),
        expected_status: DocumentStatus::OcrRequired,
        expected_is_deleted: false,
        expected_content_hash: facts.source_revision.content_hash.clone(),
        terminal_status: if facts.classification.status
            == meta_store::ClassificationStatus::ResumeCandidate
        {
            DocumentStatus::Searchable
        } else {
            DocumentStatus::Excluded
        },
        terminal_is_deleted: false,
    }];
    let projected_document = (facts.classification.status
        == meta_store::ClassificationStatus::ResumeCandidate)
        .then(|| {
            let mut document = facts.document.clone();
            document.status = DocumentStatus::Searchable;
            document.updated_at = now;
            document
        });
    let publication_documents = projected_document.into_iter().collect::<Vec<_>>();
    let publication_documents = publication_document_map(&publication_documents)?;
    let projected_documents = bind_projected_document_snapshots(
        publication.projected_documents(),
        &publication_documents,
    )?;
    let search = SearchPublicationCommit {
        generation: fulltext.generation(),
        terminal_documents: &terminal_documents,
        projections,
        projected_documents: &projected_documents,
        vector_coverage: publication.vector_coverage(),
        now,
    };
    let commit = OcrSearchPublicationCommit {
        search,
        claimed: facts.claimed,
        source_revision: facts.source_revision,
        version: facts.version,
        classification: facts.classification,
        mentions: facts.mentions,
        email_hash: facts.email_hash,
        phone_hash: facts.phone_hash,
    };
    Ok(
        match publication
            .publication_session()
            .commit_ocr_search_publication(&commit)
            .map_err(ImportPipelineError::store)?
        {
            OcrSearchPublicationOutcome::Applied => (SearchPublicationDecision::Applied, None),
            OcrSearchPublicationOutcome::ClaimSuperseded => (
                SearchPublicationDecision::NotApplied,
                Some(OcrPublicationDecisionOutcome::ClaimSuperseded),
            ),
            OcrSearchPublicationOutcome::PublicationSuperseded => (
                SearchPublicationDecision::NotApplied,
                Some(OcrPublicationDecisionOutcome::PublicationSuperseded),
            ),
        },
    )
}
