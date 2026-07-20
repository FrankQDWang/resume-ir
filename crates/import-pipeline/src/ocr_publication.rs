use std::collections::BTreeSet;
use std::path::Path;

use index_fulltext::IndexDocument;
use meta_store::{ClaimedOcrJob, ContentDigest, OwnedMetaStore, SourceRevision, UnixTimestamp};
use resume_classifier::LinearPromotionPolicy;
use sectionizer::Sectionizer;
use text_normalizer::TextNormalizer;

use super::immutable_ingest::resume_version;
use super::migration_rebuild::ensure_ocr_publication_ready;
use super::search_artifact_cache::CurrentImportCacheMode;
use super::search_artifacts::publish_incremental_search_artifacts;
use super::search_publication::SearchPublicationTransactionOutcome;
use super::search_publication_ocr::{
    decide_ocr_search_publication, OcrPublicationDecisionOutcome, OcrPublicationFacts,
};
use super::{
    contact_hashes_from_mentions, entity_mentions_from_rules, language_set, sections_to_index,
    AdmissionDecision, ImportPipelineError, ImportResourcePolicy, Result,
    SearchPublicationVectorization, OCR_PARSE_VERSION, SCHEMA_VERSION,
};

pub fn index_claimed_ocr_text(
    data_dir: &Path,
    store: &OwnedMetaStore,
    claimed: &ClaimedOcrJob,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<OcrTextIndexOutcome> {
    index_claimed_ocr_text_with_policy(
        data_dir,
        store,
        claimed,
        ocr_text,
        confidence,
        page_count,
        now,
        &LinearPromotionPolicy::default(),
        vectorization,
    )
}

pub fn index_claimed_ocr_text_with_policy(
    data_dir: &Path,
    store: &OwnedMetaStore,
    claimed: &ClaimedOcrJob,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
    vectorization: &SearchPublicationVectorization,
) -> Result<OcrTextIndexOutcome> {
    let Some(document) = store
        .document_by_id(&claimed.job.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Err(ImportPipelineError::store_invariant());
    };

    if document.content_hash.as_deref() != Some(claimed.source_fingerprint())
        || !store
            .ocr_claim_is_current(claimed)
            .map_err(ImportPipelineError::store)?
    {
        return Ok(OcrTextIndexOutcome::Superseded);
    }
    ensure_ocr_publication_ready(store)?;

    let clean_text = TextNormalizer::normalize_text_only(ocr_text);
    let sectionizer = Sectionizer::default();
    let sections = sectionizer.sectionize(&clean_text);
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let pending_doc_ids = BTreeSet::from([document.id.as_str().to_string()]);
    let content_hash = claimed
        .source_fingerprint()
        .parse::<ContentDigest>()
        .map_err(|_| ImportPipelineError::store_invariant())?;
    let source_revision =
        SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
    let version = resume_version(
        &document,
        &source_revision,
        clean_text.clone(),
        OCR_PARSE_VERSION,
        SCHEMA_VERSION,
        language_set(&clean_text),
        page_count,
        Some(confidence.unwrap_or(0.5)),
    );
    let mentions = if admitted {
        entity_mentions_from_rules(&version.id, &clean_text)
    } else {
        Vec::new()
    };
    let pending_index_documents = if admitted {
        vec![IndexDocument {
            doc_id: document.id.to_string(),
            resume_version_id: version.id.to_string(),
            file_name: document.file_name.clone(),
            clean_text: clean_text.clone(),
            sections: sections_to_index(sections),
        }]
    } else {
        Vec::new()
    };
    let (email_hash, phone_hash) = if admitted {
        contact_hashes_from_mentions(data_dir, &mentions)?
    } else {
        (None, None)
    };
    let classification = decision.into_version_classification(version.id.clone(), now);
    let publication_session = store
        .try_acquire_search_publication_session()
        .map_err(ImportPipelineError::store)?;
    let mut non_applied_outcome = None;
    let search_publication = publish_incremental_search_artifacts(
        &publication_session,
        now,
        &classification.classifier_epoch,
        pending_index_documents,
        &pending_doc_ids,
        0,
        0,
        None,
        CurrentImportCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
        vectorization,
        |publication| {
            let (decision, outcome) = decide_ocr_search_publication(
                now,
                publication,
                OcrPublicationFacts {
                    document: &document,
                    claimed,
                    source_revision: &source_revision,
                    version: &version,
                    classification: &classification,
                    mentions: &mentions,
                    email_hash: email_hash.as_ref(),
                    phone_hash: phone_hash.as_ref(),
                },
            )?;
            non_applied_outcome = outcome;
            Ok(decision)
        },
    )?;
    match search_publication {
        SearchPublicationTransactionOutcome::Committed(search_publication) => {
            let search_publication = search_publication.release();
            Ok(OcrTextIndexOutcome::Committed(OcrTextIndexSummary {
                searchable: admitted,
                indexed_documents: search_publication.fulltext.document_count(),
            }))
        }
        SearchPublicationTransactionOutcome::NotApplied => match non_applied_outcome
            .ok_or_else(ImportPipelineError::store_invariant)?
        {
            OcrPublicationDecisionOutcome::ClaimSuperseded => Ok(OcrTextIndexOutcome::Superseded),
            OcrPublicationDecisionOutcome::PublicationSuperseded => {
                Err(ImportPipelineError::index_io())
            }
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrTextIndexSummary {
    pub searchable: bool,
    pub indexed_documents: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrTextIndexOutcome {
    Committed(OcrTextIndexSummary),
    Superseded,
}
