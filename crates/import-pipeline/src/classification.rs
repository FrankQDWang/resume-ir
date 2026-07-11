use meta_store::{
    classify_resume, ClassificationResult, ClassificationStatus, ClassifierInput,
    DocumentClassificationRecord, DocumentId, ReviewDisposition, UnixTimestamp, CLASSIFIER_EPOCH,
};
use resume_classifier::{LinearPromotionPolicy, PromotionSection};
use sectionizer::SectionChunk;

pub(crate) struct AdmissionDecision(ClassificationResult);

impl AdmissionDecision {
    pub(crate) fn after_sectionization(
        clean_text: &str,
        sections: &[SectionChunk],
        promotion: &LinearPromotionPolicy,
    ) -> Self {
        let deterministic = classify_resume(ClassifierInput::NormalizedText(clean_text));
        let section_types = sections
            .iter()
            .map(|section| match &section.section_type {
                core_domain::SectionType::Profile => PromotionSection::Profile,
                core_domain::SectionType::Contact => PromotionSection::Contact,
                core_domain::SectionType::Education => PromotionSection::Education,
                core_domain::SectionType::Experience => PromotionSection::Experience,
                core_domain::SectionType::Project => PromotionSection::Project,
                core_domain::SectionType::Skill => PromotionSection::Skill,
                core_domain::SectionType::Certificate => PromotionSection::Certificate,
                core_domain::SectionType::Other(_) => PromotionSection::OtherChunk,
            })
            .collect::<Vec<_>>();
        Self(promotion.apply(clean_text, &section_types, deterministic))
    }

    pub(crate) fn ocr_backlog() -> Self {
        Self(classify_resume(ClassifierInput::OcrBacklog))
    }

    pub(crate) fn failed() -> Self {
        Self(classify_resume(ClassifierInput::Failed))
    }

    pub(crate) fn admits_search_index(&self) -> bool {
        self.0.status() == ClassificationStatus::ResumeCandidate
    }

    pub(crate) fn into_record(
        self,
        document_id: DocumentId,
        classified_at: UnixTimestamp,
    ) -> DocumentClassificationRecord {
        let status = self.0.status();
        DocumentClassificationRecord {
            document_id,
            status,
            classifier_epoch: self.0.classifier_epoch().to_string(),
            reason_codes: self.0.reason_codes().to_vec(),
            classified_at,
            review_disposition: if status == ClassificationStatus::NeedsReview {
                ReviewDisposition::Pending
            } else {
                ReviewDisposition::NotRequired
            },
        }
    }
}

pub(crate) fn is_current(record: &DocumentClassificationRecord) -> bool {
    record.classifier_epoch == CLASSIFIER_EPOCH
        || record
            .classifier_epoch
            .strip_prefix(resume_classifier::PROMOTED_EPOCH_PREFIX)
            .is_some_and(|suffix| {
                suffix.len() == 12 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
}
