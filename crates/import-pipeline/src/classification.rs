use meta_store::{
    classify_resume, ClassificationResult, ClassificationStatus, ClassifierInput,
    DocumentClassificationRecord, DocumentId, ReviewDisposition, UnixTimestamp, CLASSIFIER_EPOCH,
};
use sectionizer::SectionChunk;

pub(crate) struct AdmissionDecision(ClassificationResult);

impl AdmissionDecision {
    pub(crate) fn after_sectionization(clean_text: &str, _sections: &[SectionChunk]) -> Self {
        Self(classify_resume(ClassifierInput::NormalizedText(clean_text)))
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
}
