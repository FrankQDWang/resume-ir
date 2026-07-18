use meta_store::{
    classify_resume, ClassificationResult, ClassificationStatus, ClassifierInput,
    ResumeVersionClassification, ResumeVersionId, ReviewDisposition, SourceRevisionId,
    SourceRevisionTriage, UnixTimestamp,
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

    pub(crate) fn ocr_backlog(promotion: &LinearPromotionPolicy) -> Self {
        Self(promotion.apply("", &[], classify_resume(ClassifierInput::OcrBacklog)))
    }

    pub(crate) fn failed(promotion: &LinearPromotionPolicy) -> Self {
        Self(promotion.apply("", &[], classify_resume(ClassifierInput::Failed)))
    }

    pub(crate) fn admits_search_index(&self) -> bool {
        self.0.status() == ClassificationStatus::ResumeCandidate
    }

    pub(crate) fn into_version_classification(
        self,
        resume_version_id: ResumeVersionId,
        classified_at: UnixTimestamp,
    ) -> ResumeVersionClassification {
        let status = self.0.status();
        ResumeVersionClassification {
            resume_version_id,
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

    pub(crate) fn into_source_triage(
        self,
        source_revision_id: SourceRevisionId,
        triaged_at: UnixTimestamp,
    ) -> SourceRevisionTriage {
        SourceRevisionTriage {
            source_revision_id,
            status: self.0.status(),
            triage_epoch: self.0.classifier_epoch().to_string(),
            reason_codes: self.0.reason_codes().to_vec(),
            triaged_at,
        }
    }
}

#[cfg(test)]
#[path = "classification_tests.rs"]
mod tests;
