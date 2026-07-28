use std::path::Path;

use core_domain::EntityMentionId;
use extractor_rules::{extract_strong_fields, FieldType, RuleEvidenceKind, RuleMatch};
use index_fulltext::IndexDocument;
use meta_store::{
    ContactHash, Document, DocumentStatus, EntityMention, EntityType, OwnedMetaStore,
    ResumeVersion, ResumeVersionId, SourceRevision, UnixTimestamp,
};
use privacy::{ContactHasher, ContactKind};
use resume_classifier::LinearPromotionPolicy;

use super::model::{PendingSearchableDocument, PendingSearchablePublicationKind};
use crate::classification::AdmissionDecision;
use crate::immutable_ingest::{self, StagedDerivedData, StagedResume};
use crate::processing_contract::current_source_triage_epoch;
use crate::source_dispositions::ProcessedFile;
use crate::{ImportPipelineError, Result};

pub(crate) fn contact_hashes_from_mentions(
    data_dir: &Path,
    mentions: &[EntityMention],
) -> Result<(Option<ContactHash>, Option<ContactHash>)> {
    let email = best_normalized_contact(mentions, EntityType::Email);
    let phone = best_normalized_contact(mentions, EntityType::Phone);
    if email.is_none() && phone.is_none() {
        return Ok((None, None));
    }

    let hasher = ContactHasher::load_or_create(data_dir).map_err(ImportPipelineError::privacy)?;
    let email_hash = email
        .map(|value| hasher.hash_contact(ContactKind::Email, value))
        .transpose()
        .map_err(ImportPipelineError::privacy)?;
    let phone_hash = phone
        .map(|value| hasher.hash_contact(ContactKind::Phone, value))
        .transpose()
        .map_err(ImportPipelineError::privacy)?;

    Ok((email_hash, phone_hash))
}

pub(crate) fn best_normalized_contact(
    mentions: &[EntityMention],
    entity_type: EntityType,
) -> Option<&str> {
    let mut candidates = mentions
        .iter()
        .filter(|mention| mention.entity_type == entity_type)
        .filter_map(|mention| {
            let normalized = mention.normalized_value.as_deref()?;
            Some((
                normalized,
                mention.confidence,
                mention.span_start.unwrap_or(usize::MAX),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.cmp(right.0))
    });
    candidates.first().map(|candidate| candidate.0)
}

pub(crate) fn prepare_pending_searchable_document(
    data_dir: &Path,
    document: Document,
    source_revision: SourceRevision,
    decision: AdmissionDecision,
    version: ResumeVersion,
    mentions: Vec<EntityMention>,
    index_document: IndexDocument,
    now: UnixTimestamp,
) -> Result<ProcessedFile> {
    let classification = decision.into_version_classification(version.id.clone(), now);
    let (email_hash, phone_hash) = contact_hashes_from_mentions(data_dir, &mentions)?;
    Ok(ProcessedFile::Searchable {
        pending: Box::new(PendingSearchableDocument {
            document,
            source_revision,
            classification,
            version,
            mentions,
            email_hash,
            phone_hash,
            index_document,
            publication_kind: PendingSearchablePublicationKind::Replacement,
            source_occurrence: None,
        }),
    })
}

pub(crate) fn persist_non_searchable(
    store: &OwnedMetaStore,
    document: &Document,
    source_revision: &SourceRevision,
    version: &ResumeVersion,
    decision: AdmissionDecision,
    now: UnixTimestamp,
) -> Result<()> {
    let classification = decision.into_version_classification(version.id.clone(), now);
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::ClassifiedVersion {
                version,
                classification: &classification,
                mentions: &[],
                email_hash: None,
                phone_hash: None,
            },
        },
    )
    .map_err(ImportPipelineError::store)
}

pub(crate) fn persist_document_failure_without_revision(
    store: &OwnedMetaStore,
    document: &Document,
) -> Result<()> {
    let has_active_projection = store
        .active_search_projection_for_document(&document.id)
        .map_err(ImportPipelineError::store)?
        .is_some();
    if has_active_projection {
        return Ok(());
    }
    store
        .upsert_document(document)
        .map_err(ImportPipelineError::store)
}

pub(crate) fn persist_source_revision_failure(
    store: &OwnedMetaStore,
    document: &Document,
    source_revision: &SourceRevision,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    let triage_epoch = current_source_triage_epoch(linear_promotion)?;
    let triage = AdmissionDecision::failed(linear_promotion).into_source_triage(
        source_revision.id.clone(),
        &triage_epoch,
        now,
    );
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::SourceTriage(&triage),
        },
    )
    .map_err(ImportPipelineError::store)
}

pub(crate) fn mark_ocr_required_and_enqueue(
    store: &OwnedMetaStore,
    document: &mut Document,
    source_revision: &SourceRevision,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<bool> {
    document.status = DocumentStatus::OcrRequired;
    document.updated_at = now;
    let triage_epoch = current_source_triage_epoch(linear_promotion)?;
    let triage = AdmissionDecision::ocr_backlog(linear_promotion).into_source_triage(
        source_revision.id.clone(),
        &triage_epoch,
        now,
    );
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::SourceTriage(&triage),
        },
    )
    .map_err(ImportPipelineError::store)?;
    let enqueue = store
        .enqueue_ocr_job_for_source_triage(&source_revision.id, &triage_epoch, now)
        .map_err(ImportPipelineError::store)?;

    Ok(enqueue.scheduled)
}

pub(crate) fn entity_mentions_from_rules(
    version_id: &ResumeVersionId,
    clean_text: &str,
) -> Vec<EntityMention> {
    extract_strong_fields(clean_text)
        .into_iter()
        .enumerate()
        .map(|(index, field)| entity_mention_from_rule(version_id, index, field))
        .collect()
}

pub(crate) fn entity_mention_from_rule(
    version_id: &ResumeVersionId,
    index: usize,
    field: RuleMatch,
) -> EntityMention {
    let evidence_kind = field.evidence_kind();
    let (raw_value, span_start, span_end, extractor) = match evidence_kind {
        RuleEvidenceKind::SourceSpan => (
            field.raw_value,
            Some(field.span_start),
            Some(field.span_end),
            "rules-v2",
        ),
        RuleEvidenceKind::DerivedAggregate => (
            field.normalized_value.clone().unwrap_or(field.raw_value),
            None,
            None,
            "rules-v2-derived",
        ),
    };
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[
            extractor,
            version_id.as_str(),
            &index.to_string(),
        ]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type: entity_type_from_field_type(&field.field_type),
        raw_value,
        normalized_value: field.normalized_value,
        span_start,
        span_end,
        confidence: field.confidence,
        extractor: extractor.to_string(),
    }
}

pub(crate) fn entity_type_from_field_type(field_type: &FieldType) -> EntityType {
    match field_type {
        FieldType::Name => EntityType::Name,
        FieldType::Email => EntityType::Email,
        FieldType::Phone => EntityType::Phone,
        FieldType::WeChat => EntityType::WeChat,
        FieldType::DateRange => EntityType::DateRange,
        FieldType::School => EntityType::School,
        FieldType::SchoolTier => EntityType::SchoolTier,
        FieldType::Degree => EntityType::Degree,
        FieldType::Major => EntityType::Major,
        FieldType::Company => EntityType::Company,
        FieldType::Title => EntityType::Title,
        FieldType::Location => EntityType::Location,
        FieldType::Skill => EntityType::Skill,
        FieldType::Certificate => EntityType::Certificate,
        FieldType::YearsExperience => EntityType::YearsExperience,
    }
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

#[cfg(test)]
mod source_triage_contract_tests {
    use super::*;
    use meta_store::{
        ClassificationStatus, ContentDigest, DataDirectoryOwnerAcquisition,
        DataDirectoryOwnerLease, DocumentId, FileExtension, ImmutableIngestStage, ReasonCode,
        SourceRevisionTriage, CLASSIFIER_EPOCH,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parser_routing_change_uses_a_new_source_triage_identity() {
        let data_dir = TestDir::new("source-triage-processing-contract-identity");
        let owner = match DataDirectoryOwnerLease::try_acquire(data_dir.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
        };
        let store = owner.open_store().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_210_100);
        let content_hash = ContentDigest::from_bytes(b"synthetic parser routing change");
        let mut document = Document {
            id: DocumentId::from_non_secret_parts(&["source-triage-processing-contract-identity"]),
            source_uri: "file:///fixture/synthetic.pdf".to_string(),
            normalized_path: "/fixture/synthetic.pdf".to_string(),
            file_name: "synthetic.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 128,
            mtime: now,
            content_hash: Some(content_hash.as_str().to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::FailedPermanent,
        };
        let source_revision =
            SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
        let legacy_triage = SourceRevisionTriage {
            source_revision_id: source_revision.id.clone(),
            status: ClassificationStatus::Failed,
            triage_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::ParserFailed],
            triaged_at: now,
        };
        store
            .stage_immutable_ingest(ImmutableIngestStage::SourceTriage {
                document: &document,
                source_revision: &source_revision,
                triage: &legacy_triage,
            })
            .unwrap();

        let promotion = LinearPromotionPolicy::default();
        let current_epoch = current_source_triage_epoch(&promotion).unwrap();
        let queued =
            mark_ocr_required_and_enqueue(&store, &mut document, &source_revision, now, &promotion)
                .unwrap();

        assert!(queued);
        assert_eq!(
            store
                .source_revision_triage(&source_revision.id, CLASSIFIER_EPOCH)
                .unwrap(),
            Some(legacy_triage)
        );
        let current = store
            .source_revision_triage(&source_revision.id, current_epoch.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(current.status, ClassificationStatus::OcrBacklog);
        assert_eq!(current.reason_codes, vec![ReasonCode::OcrRequired]);
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
