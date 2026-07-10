use std::fmt;

pub use resume_classifier::{ClassificationStatus, ReasonCode};
use rusqlite::{params, OptionalExtension};

use super::{
    document_status_to_storage, i64_to_u64, DocumentId, DocumentStatus, MetaStore, MetaStoreError,
    Result, UnixTimestamp,
};

const DOCUMENT_CLASSIFICATION_REASON_LIMIT: usize = 8;
const _: [(); DOCUMENT_CLASSIFICATION_REASON_LIMIT] = [(); resume_classifier::MAX_REASON_CODES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDisposition {
    NotRequired,
    Pending,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DocumentClassificationRecord {
    pub document_id: DocumentId,
    pub status: ClassificationStatus,
    pub classifier_epoch: String,
    pub reason_codes: Vec<ReasonCode>,
    pub classified_at: UnixTimestamp,
    pub review_disposition: ReviewDisposition,
}

impl fmt::Debug for DocumentClassificationRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentClassificationRecord")
            .field("document_id", &"<redacted>")
            .field("status", &self.status)
            .field("classifier_epoch", &"<redacted>")
            .field("reason_count", &self.reason_codes.len())
            .field("review_disposition", &self.review_disposition)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocumentClassificationCounts {
    pub resume_candidate: u64,
    pub non_resume: u64,
    pub needs_review: u64,
    pub ocr_backlog: u64,
    pub failed: u64,
}

impl MetaStore {
    pub fn upsert_document_classification(
        &self,
        record: &DocumentClassificationRecord,
    ) -> Result<()> {
        validate_record(record)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT INTO document_classification (
                    document_id, status, classifier_epoch, classified_at_seconds,
                    review_disposition
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(document_id) DO UPDATE SET
                    status = excluded.status,
                    classifier_epoch = excluded.classifier_epoch,
                    classified_at_seconds = excluded.classified_at_seconds,
                    review_disposition = excluded.review_disposition",
                params![
                    record.document_id.as_str(),
                    record.status.as_str(),
                    record.classifier_epoch,
                    record.classified_at.as_unix_seconds(),
                    review_disposition_to_storage(record.review_disposition),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM document_classification_reason WHERE document_id = ?1",
                params![record.document_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        for (ordinal, reason_code) in record.reason_codes.iter().copied().enumerate() {
            transaction
                .execute(
                    "INSERT INTO document_classification_reason (
                        document_id, ordinal, reason_code
                     ) VALUES (?1, ?2, ?3)",
                    params![
                        record.document_id.as_str(),
                        ordinal,
                        reason_code_to_storage(reason_code),
                    ],
                )
                .map_err(MetaStoreError::storage)?;
        }
        transaction.commit().map_err(MetaStoreError::storage)
    }

    pub fn document_classification_by_id(
        &self,
        document_id: &DocumentId,
    ) -> Result<Option<DocumentClassificationRecord>> {
        let connection = self.connection.borrow();
        let parent = connection
            .query_row(
                "SELECT status, classifier_epoch, classified_at_seconds, review_disposition
                 FROM document_classification WHERE document_id = ?1",
                params![document_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        let Some((status, classifier_epoch, classified_at_seconds, review_disposition)) = parent
        else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT ordinal, reason_code FROM document_classification_reason
                 WHERE document_id = ?1 ORDER BY ordinal",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut reason_codes = Vec::new();
        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            let ordinal = row.get::<_, i64>(0).map_err(MetaStoreError::storage)?;
            if ordinal != reason_codes.len() as i64 {
                return Err(MetaStoreError::invalid_value(
                    "document_classification_reason.ordinal",
                ));
            }
            let reason_code = row.get::<_, String>(1).map_err(MetaStoreError::storage)?;
            reason_codes.push(reason_code_from_storage(&reason_code)?);
        }

        let record = DocumentClassificationRecord {
            document_id: document_id.clone(),
            status: classification_status_from_storage(&status)?,
            classifier_epoch,
            reason_codes,
            classified_at: UnixTimestamp::from_unix_seconds(classified_at_seconds),
            review_disposition: review_disposition_from_storage(&review_disposition)?,
        };
        validate_record(&record)?;
        Ok(Some(record))
    }

    pub fn document_classification_counts(&self) -> Result<DocumentClassificationCounts> {
        let connection = self.connection.borrow();
        let counts = connection
            .query_row(
                "SELECT
                    COALESCE(SUM(classification.status = 'resume_candidate'), 0),
                    COALESCE(SUM(classification.status = 'non_resume'), 0),
                    COALESCE(SUM(classification.status = 'needs_review'), 0),
                    COALESCE(SUM(classification.status = 'ocr_backlog'), 0),
                    COALESCE(SUM(classification.status = 'failed'), 0)
                 FROM document_classification AS classification
                 JOIN document ON document.id = classification.document_id
                 WHERE document.is_deleted = 0 AND document.status <> ?1",
                params![document_status_to_storage(DocumentStatus::Deleted)],
                |row| {
                    Ok([
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ])
                },
            )
            .map_err(MetaStoreError::storage)?;
        Ok(DocumentClassificationCounts {
            resume_candidate: i64_to_u64(counts[0], "document_classification.count")?,
            non_resume: i64_to_u64(counts[1], "document_classification.count")?,
            needs_review: i64_to_u64(counts[2], "document_classification.count")?,
            ocr_backlog: i64_to_u64(counts[3], "document_classification.count")?,
            failed: i64_to_u64(counts[4], "document_classification.count")?,
        })
    }
}

fn validate_record(record: &DocumentClassificationRecord) -> Result<()> {
    let epoch = record.classifier_epoch.as_bytes();
    if epoch.is_empty()
        || epoch.len() > 64
        || !epoch
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        return Err(MetaStoreError::invalid_value(
            "document_classification.classifier_epoch",
        ));
    }
    if record.reason_codes.is_empty()
        || record.reason_codes.len() > DOCUMENT_CLASSIFICATION_REASON_LIMIT
        || record
            .reason_codes
            .iter()
            .enumerate()
            .any(|(index, reason)| record.reason_codes[..index].contains(reason))
    {
        return Err(MetaStoreError::invalid_value(
            "document_classification.reason_codes",
        ));
    }
    let status_marker_valid = matches!(
        (record.status, record.reason_codes.as_slice()),
        (
            ClassificationStatus::ResumeCandidate,
            [ReasonCode::CorroboratedResumeSignals, ..]
        ) | (
            ClassificationStatus::NonResume,
            [ReasonCode::CorroboratedNonResumeSignals, ..]
        ) | (
            ClassificationStatus::NeedsReview,
            [
                ReasonCode::ConflictingSignalFamilies | ReasonCode::InsufficientSignalFamilies,
                ..
            ],
        ) | (ClassificationStatus::OcrBacklog, [ReasonCode::OcrRequired])
            | (
                ClassificationStatus::Failed,
                [ReasonCode::EmptyNormalizedText | ReasonCode::ParserFailed],
            )
    );
    if !status_marker_valid {
        return Err(MetaStoreError::invalid_value(
            "document_classification.reason_codes",
        ));
    }
    if record.review_disposition != review_disposition_for_status(record.status) {
        return Err(MetaStoreError::invalid_value(
            "document_classification.review_disposition",
        ));
    }
    Ok(())
}

fn review_disposition_for_status(status: ClassificationStatus) -> ReviewDisposition {
    match status {
        ClassificationStatus::NeedsReview => ReviewDisposition::Pending,
        ClassificationStatus::ResumeCandidate
        | ClassificationStatus::NonResume
        | ClassificationStatus::OcrBacklog
        | ClassificationStatus::Failed => ReviewDisposition::NotRequired,
    }
}

fn review_disposition_to_storage(disposition: ReviewDisposition) -> &'static str {
    match disposition {
        ReviewDisposition::NotRequired => "not_required",
        ReviewDisposition::Pending => "pending",
    }
}

fn review_disposition_from_storage(value: &str) -> Result<ReviewDisposition> {
    match value {
        "not_required" => Ok(ReviewDisposition::NotRequired),
        "pending" => Ok(ReviewDisposition::Pending),
        _ => Err(MetaStoreError::invalid_value(
            "document_classification.review_disposition",
        )),
    }
}

fn classification_status_from_storage(value: &str) -> Result<ClassificationStatus> {
    match value {
        "resume_candidate" => Ok(ClassificationStatus::ResumeCandidate),
        "non_resume" => Ok(ClassificationStatus::NonResume),
        "needs_review" => Ok(ClassificationStatus::NeedsReview),
        "ocr_backlog" => Ok(ClassificationStatus::OcrBacklog),
        "failed" => Ok(ClassificationStatus::Failed),
        _ => Err(MetaStoreError::invalid_value(
            "document_classification.status",
        )),
    }
}

macro_rules! reason_code_storage {
    ($($variant:ident => $stored:literal),+ $(,)?) => {
        fn reason_code_to_storage(reason: ReasonCode) -> &'static str {
            match reason { $(ReasonCode::$variant => $stored,)+ }
        }

        fn reason_code_from_storage(value: &str) -> Result<ReasonCode> {
            match value {
                $($stored => Ok(ReasonCode::$variant),)+
                _ => Err(MetaStoreError::invalid_value("document_classification_reason.reason_code")),
            }
        }
    };
}

reason_code_storage! {
    ProfileHeading => "profile_heading",
    ExperienceHeading => "experience_heading",
    EducationHeading => "education_heading",
    SkillsHeading => "skills_heading",
    CareerHistoryDetail => "career_history_detail",
    InvoiceHeading => "invoice_heading",
    InvoiceTerms => "invoice_terms",
    MeetingHeading => "meeting_heading",
    MeetingWorkflow => "meeting_workflow",
    ManualHeading => "manual_heading",
    ManualInstructions => "manual_instructions",
    CorroboratedResumeSignals => "corroborated_resume_signals",
    CorroboratedNonResumeSignals => "corroborated_non_resume_signals",
    ConflictingSignalFamilies => "conflicting_signal_families",
    InsufficientSignalFamilies => "insufficient_signal_families",
    EmptyNormalizedText => "empty_normalized_text",
    OcrRequired => "ocr_required",
    ParserFailed => "parser_failed",
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, FileExtension};
    use ClassificationStatus as Status;
    use ReasonCode as Reason;

    fn document(label: &str) -> Document {
        let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
        Document {
            id: DocumentId::from_non_secret_parts(&["classification", label]),
            source_uri: format!("synthetic://classification/{label}"),
            normalized_path: format!("synthetic/classification/{label}.txt"),
            file_name: format!("{label}.txt"),
            extension: FileExtension::Txt,
            byte_size: 128,
            mtime: now,
            content_hash: None,
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::IndexedPartial,
        }
    }

    fn record(
        document_id: DocumentId,
        status: Status,
        reason_code: Reason,
    ) -> DocumentClassificationRecord {
        DocumentClassificationRecord {
            document_id,
            status,
            classifier_epoch: "precision_first_v1".to_string(),
            reason_codes: vec![reason_code],
            classified_at: UnixTimestamp::from_unix_seconds(1_800_000_001),
            review_disposition: review_disposition_for_status(status),
        }
    }

    fn assert_stored(store: &MetaStore, expected: &DocumentClassificationRecord) {
        let actual = store
            .document_classification_by_id(&expected.document_id)
            .unwrap();
        assert_eq!(actual.as_ref(), Some(expected));
    }

    #[test]
    fn five_states_round_trip_and_visible_counts_exclude_soft_deleted_records() {
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let cases = [
            (Status::ResumeCandidate, Reason::CorroboratedResumeSignals),
            (Status::NonResume, Reason::CorroboratedNonResumeSignals),
            (Status::NeedsReview, Reason::ConflictingSignalFamilies),
            (Status::OcrBacklog, Reason::OcrRequired),
            (Status::Failed, Reason::ParserFailed),
        ];
        let mut records = Vec::new();
        for (index, (status, reason)) in cases.into_iter().enumerate() {
            let document = document(&format!("state-{index}"));
            store.upsert_document(&document).unwrap();
            let record = record(document.id, status, reason);
            store.upsert_document_classification(&record).unwrap();
            assert_stored(&store, &record);
            records.push(record);
        }

        assert_eq!(
            store.document_classification_counts().unwrap(),
            DocumentClassificationCounts {
                resume_candidate: 1,
                non_resume: 1,
                needs_review: 1,
                ocr_backlog: 1,
                failed: 1,
            }
        );
        store
            .mark_document_deleted(
                &records[0].document_id,
                UnixTimestamp::from_unix_seconds(1_800_000_002),
            )
            .unwrap();
        assert_eq!(
            store
                .document_classification_counts()
                .unwrap()
                .resume_candidate,
            0
        );
        assert_stored(&store, &records.remove(0));
    }

    #[test]
    fn atomic_upsert_replaces_reasons_and_rejects_invalid_or_missing_records() {
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let document = document("atomic-upsert");
        store.upsert_document(&document).unwrap();
        let original = record(
            document.id.clone(),
            Status::ResumeCandidate,
            Reason::CorroboratedResumeSignals,
        );
        store.upsert_document_classification(&original).unwrap();
        let mut replacement = record(
            document.id.clone(),
            Status::NeedsReview,
            Reason::ConflictingSignalFamilies,
        );
        replacement.classifier_epoch = "precision_first_v2".to_string();
        replacement
            .reason_codes
            .push(Reason::InsufficientSignalFamilies);
        store.upsert_document_classification(&replacement).unwrap();
        assert_stored(&store, &replacement);

        for epoch in [String::new(), "free-form".to_string(), "x".repeat(65)] {
            let invalid = DocumentClassificationRecord {
                classifier_epoch: epoch,
                ..replacement.clone()
            };
            assert!(store.upsert_document_classification(&invalid).is_err());
        }
        for reason_codes in [
            vec![],
            vec![Reason::ExperienceHeading],
            vec![Reason::OcrRequired; 2],
            vec![
                Reason::ProfileHeading,
                Reason::ExperienceHeading,
                Reason::EducationHeading,
                Reason::SkillsHeading,
                Reason::CareerHistoryDetail,
                Reason::InvoiceHeading,
                Reason::InvoiceTerms,
                Reason::MeetingHeading,
                Reason::MeetingWorkflow,
            ],
        ] {
            let invalid = DocumentClassificationRecord {
                reason_codes,
                ..replacement.clone()
            };
            assert!(store.upsert_document_classification(&invalid).is_err());
        }
        let invalid = DocumentClassificationRecord {
            review_disposition: ReviewDisposition::NotRequired,
            ..replacement.clone()
        };
        assert!(store.upsert_document_classification(&invalid).is_err());
        assert_stored(&store, &replacement);

        let missing = record(
            DocumentId::from_non_secret_parts(&["classification", "missing"]),
            Status::Failed,
            Reason::ParserFailed,
        );
        assert!(store.upsert_document_classification(&missing).is_err());
        let debug = format!("{replacement:?}");
        assert!(!debug.contains(replacement.document_id.as_str()));
        assert!(!debug.contains("precision_first_v2"));
        assert!(!debug.contains("ConflictingSignalFamilies"));
    }
}
