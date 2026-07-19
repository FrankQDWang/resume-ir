use std::fmt;

pub use resume_classifier::{ClassificationStatus, ReasonCode};
use rusqlite::{params, Connection, OptionalExtension};

use super::{
    i64_to_u64, IdentityInsertOutcome, MetaStoreError, MetadataStore, MetadataStoreAccess,
    MetadataStoreWriteAccess, Result, ResumeVersionId, SourceRevisionId, UnixTimestamp,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CurrentClassifierEpoch<'a> {
    value: &'a str,
    source: ClassifierEpochSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassifierEpochSource {
    Deterministic,
    LocalLinearPromotion,
}

impl<'a> CurrentClassifierEpoch<'a> {
    pub fn parse(value: &'a str) -> Option<Self> {
        let source = if value == resume_classifier::CLASSIFIER_EPOCH {
            ClassifierEpochSource::Deterministic
        } else {
            let suffix = value.strip_prefix(resume_classifier::PROMOTED_EPOCH_PREFIX)?;
            if suffix.len() != 12
                || !suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            {
                return None;
            }
            ClassifierEpochSource::LocalLinearPromotion
        };
        Some(Self { value, source })
    }

    pub fn as_str(self) -> &'a str {
        self.value
    }

    pub fn source(self) -> ClassifierEpochSource {
        self.source
    }
}

impl fmt::Debug for CurrentClassifierEpoch<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentClassifierEpoch")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

const CLASSIFICATION_REASON_LIMIT: usize = 8;
const _: [(); CLASSIFICATION_REASON_LIMIT] = [(); resume_classifier::MAX_REASON_CODES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDisposition {
    NotRequired,
    Pending,
}

/// Immutable source-level routing decision made before normalized text exists.
///
/// This record may only route a source revision to OCR or record a parser
/// failure. Resume/non-resume decisions always belong to a concrete immutable
/// resume version.
#[derive(Clone, PartialEq, Eq)]
pub struct SourceRevisionTriage {
    pub source_revision_id: SourceRevisionId,
    pub status: ClassificationStatus,
    pub triage_epoch: String,
    pub reason_codes: Vec<ReasonCode>,
    pub triaged_at: UnixTimestamp,
}

impl fmt::Debug for SourceRevisionTriage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRevisionTriage")
            .field("source_revision_id", &self.source_revision_id)
            .field("status", &self.status)
            .field("triage_epoch", &"<redacted>")
            .field("reason_count", &self.reason_codes.len())
            .finish()
    }
}

/// Immutable final classification of one exact normalized resume version.
#[derive(Clone, PartialEq, Eq)]
pub struct ResumeVersionClassification {
    pub resume_version_id: ResumeVersionId,
    pub status: ClassificationStatus,
    pub classifier_epoch: String,
    pub reason_codes: Vec<ReasonCode>,
    pub classified_at: UnixTimestamp,
    pub review_disposition: ReviewDisposition,
}

impl fmt::Debug for ResumeVersionClassification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeVersionClassification")
            .field("resume_version_id", &self.resume_version_id)
            .field("status", &self.status)
            .field("classifier_epoch", &"<redacted>")
            .field("reason_count", &self.reason_codes.len())
            .field("review_disposition", &self.review_disposition)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassificationCounts {
    pub resume_candidate: u64,
    pub non_resume: u64,
    pub needs_review: u64,
    pub ocr_backlog: u64,
    /// Final version failures plus source-level triage failures.
    pub failed: u64,
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn insert_source_revision_triage(
        &self,
        triage: &SourceRevisionTriage,
    ) -> Result<IdentityInsertOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let outcome = insert_source_revision_triage_in_connection(&transaction, triage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    pub fn source_revision_triage(
        &self,
        source_revision_id: &SourceRevisionId,
        triage_epoch: &str,
    ) -> Result<Option<SourceRevisionTriage>> {
        source_revision_triage_from_connection(
            &self.connection.borrow(),
            source_revision_id,
            triage_epoch,
        )
    }

    pub fn insert_resume_version_classification(
        &self,
        classification: &ResumeVersionClassification,
    ) -> Result<IdentityInsertOutcome>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let outcome =
            insert_resume_version_classification_in_connection(&transaction, classification)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    pub fn resume_version_classification(
        &self,
        resume_version_id: &ResumeVersionId,
        classifier_epoch: &str,
    ) -> Result<Option<ResumeVersionClassification>> {
        resume_version_classification_from_connection(
            &self.connection.borrow(),
            resume_version_id,
            classifier_epoch,
        )
    }

    pub fn resume_version_has_current_resume_candidate_classification(
        &self,
        resume_version_id: &ResumeVersionId,
    ) -> Result<bool> {
        resume_version_has_current_resume_candidate_classification_in_connection(
            &self.connection.borrow(),
            resume_version_id,
        )
    }

    pub fn resume_version_has_resume_candidate_classification_at_epoch(
        &self,
        resume_version_id: &ResumeVersionId,
        classifier_epoch: &str,
    ) -> Result<bool> {
        resume_version_has_resume_candidate_classification_at_epoch_in_connection(
            &self.connection.borrow(),
            resume_version_id,
            classifier_epoch,
        )
    }

    pub fn classification_counts(&self, classifier_epoch: &str) -> Result<ClassificationCounts> {
        if CurrentClassifierEpoch::parse(classifier_epoch).is_none() {
            return Err(MetaStoreError::invalid_value(
                "classification_counts.classifier_epoch",
            ));
        }
        let counts = self
            .connection
            .borrow()
            .query_row(
                "WITH final AS (
                    SELECT classification.status
                    FROM resume_version_classification AS classification
                    JOIN resume_version AS version
                      ON version.id = classification.resume_version_id
                    JOIN source_revision AS revision
                      ON revision.id = version.source_revision_id
                    JOIN document
                      ON document.id = revision.document_id
                     AND document.content_hash = revision.content_hash
                    WHERE document.is_deleted = 0
                      AND classification.classifier_epoch = ?1
                 ), triage AS (
                    SELECT source_triage.status
                    FROM source_revision_triage AS source_triage
                    JOIN source_revision AS revision
                      ON revision.id = source_triage.source_revision_id
                    JOIN document
                      ON document.id = revision.document_id
                     AND document.content_hash = revision.content_hash
                    WHERE document.is_deleted = 0
                      AND source_triage.triage_epoch = ?1
                 )
                 SELECT
                    COALESCE((SELECT SUM(status = 'resume_candidate') FROM final), 0),
                    COALESCE((SELECT SUM(status = 'non_resume') FROM final), 0),
                    COALESCE((SELECT SUM(status = 'needs_review') FROM final), 0),
                    COALESCE((SELECT SUM(status = 'ocr_backlog') FROM triage), 0),
                    COALESCE((SELECT SUM(status = 'failed') FROM final), 0)
                      + COALESCE((SELECT SUM(status = 'failed') FROM triage), 0)",
                params![classifier_epoch],
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
        Ok(ClassificationCounts {
            resume_candidate: i64_to_u64(counts[0], "classification.count")?,
            non_resume: i64_to_u64(counts[1], "classification.count")?,
            needs_review: i64_to_u64(counts[2], "classification.count")?,
            ocr_backlog: i64_to_u64(counts[3], "classification.count")?,
            failed: i64_to_u64(counts[4], "classification.count")?,
        })
    }
}

pub(super) fn insert_source_revision_triage_in_connection(
    connection: &Connection,
    triage: &SourceRevisionTriage,
) -> Result<IdentityInsertOutcome> {
    validate_source_revision_triage(triage)?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO source_revision_triage (
                source_revision_id, status, triage_epoch, triaged_at_seconds
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                triage.source_revision_id.as_str(),
                triage.status.as_str(),
                triage.triage_epoch,
                triage.triaged_at.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 0 {
        return match source_revision_triage_from_connection(
            connection,
            &triage.source_revision_id,
            &triage.triage_epoch,
        )? {
            Some(existing) if existing == *triage => Ok(IdentityInsertOutcome::AlreadyPresent),
            Some(_) => Err(MetaStoreError::immutable_identity_conflict(
                "source_revision_triage",
            )),
            None => Err(MetaStoreError::storage_invariant()),
        };
    }
    for (ordinal, reason_code) in triage.reason_codes.iter().copied().enumerate() {
        connection
            .execute(
                "INSERT INTO source_revision_triage_reason (
                    source_revision_id, triage_epoch, ordinal, reason_code
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    triage.source_revision_id.as_str(),
                    triage.triage_epoch,
                    ordinal,
                    reason_code_to_storage(reason_code),
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    Ok(IdentityInsertOutcome::Inserted)
}

pub(super) fn insert_resume_version_classification_in_connection(
    connection: &Connection,
    classification: &ResumeVersionClassification,
) -> Result<IdentityInsertOutcome> {
    validate_resume_version_classification(classification)?;
    let changed = connection
        .execute(
            "INSERT OR IGNORE INTO resume_version_classification (
                resume_version_id, status, classifier_epoch, classified_at_seconds,
                review_disposition
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                classification.resume_version_id.as_str(),
                classification.status.as_str(),
                classification.classifier_epoch,
                classification.classified_at.as_unix_seconds(),
                review_disposition_to_storage(classification.review_disposition),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 0 {
        return match resume_version_classification_from_connection(
            connection,
            &classification.resume_version_id,
            &classification.classifier_epoch,
        )? {
            Some(existing) if existing == *classification => {
                Ok(IdentityInsertOutcome::AlreadyPresent)
            }
            Some(_) => Err(MetaStoreError::immutable_identity_conflict(
                "resume_version_classification",
            )),
            None => Err(MetaStoreError::storage_invariant()),
        };
    }
    for (ordinal, reason_code) in classification.reason_codes.iter().copied().enumerate() {
        connection
            .execute(
                "INSERT INTO resume_version_classification_reason (
                    resume_version_id, classifier_epoch, ordinal, reason_code
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    classification.resume_version_id.as_str(),
                    classification.classifier_epoch,
                    ordinal,
                    reason_code_to_storage(reason_code),
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    Ok(IdentityInsertOutcome::Inserted)
}

pub(super) fn source_revision_triage_from_connection(
    connection: &Connection,
    source_revision_id: &SourceRevisionId,
    triage_epoch: &str,
) -> Result<Option<SourceRevisionTriage>> {
    let parent = connection
        .query_row(
            "SELECT status, triage_epoch, triaged_at_seconds
             FROM source_revision_triage
             WHERE source_revision_id = ?1 AND triage_epoch = ?2",
            params![source_revision_id.as_str(), triage_epoch],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let Some((status, triage_epoch, triaged_at)) = parent else {
        return Ok(None);
    };
    let reason_codes = read_reason_codes(
        connection,
        "source_revision_triage_reason",
        "source_revision_id",
        source_revision_id.as_str(),
        "triage_epoch",
        &triage_epoch,
    )?;
    let triage = SourceRevisionTriage {
        source_revision_id: source_revision_id.clone(),
        status: classification_status_from_storage(&status)?,
        triage_epoch,
        reason_codes,
        triaged_at: UnixTimestamp::from_unix_seconds(triaged_at),
    };
    validate_source_revision_triage(&triage)?;
    Ok(Some(triage))
}

pub(super) fn resume_version_classification_from_connection(
    connection: &Connection,
    resume_version_id: &ResumeVersionId,
    classifier_epoch: &str,
) -> Result<Option<ResumeVersionClassification>> {
    let parent = connection
        .query_row(
            "SELECT status, classifier_epoch, classified_at_seconds, review_disposition
             FROM resume_version_classification
             WHERE resume_version_id = ?1 AND classifier_epoch = ?2",
            params![resume_version_id.as_str(), classifier_epoch],
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
    let Some((status, classifier_epoch, classified_at, review_disposition)) = parent else {
        return Ok(None);
    };
    let reason_codes = read_reason_codes(
        connection,
        "resume_version_classification_reason",
        "resume_version_id",
        resume_version_id.as_str(),
        "classifier_epoch",
        &classifier_epoch,
    )?;
    let classification = ResumeVersionClassification {
        resume_version_id: resume_version_id.clone(),
        status: classification_status_from_storage(&status)?,
        classifier_epoch,
        reason_codes,
        classified_at: UnixTimestamp::from_unix_seconds(classified_at),
        review_disposition: review_disposition_from_storage(&review_disposition)?,
    };
    validate_resume_version_classification(&classification)?;
    Ok(Some(classification))
}

fn read_reason_codes(
    connection: &Connection,
    table: &'static str,
    identity_column: &'static str,
    identity: &str,
    epoch_column: &'static str,
    epoch: &str,
) -> Result<Vec<ReasonCode>> {
    let sql = format!(
        "SELECT ordinal, reason_code FROM {table}
         WHERE {identity_column} = ?1 AND {epoch_column} = ?2 ORDER BY ordinal"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![identity, epoch])
        .map_err(MetaStoreError::storage)?;
    let mut reason_codes = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        if row.get::<_, i64>(0).map_err(MetaStoreError::storage)? != reason_codes.len() as i64 {
            return Err(MetaStoreError::invalid_value(
                "classification_reason.ordinal",
            ));
        }
        reason_codes.push(reason_code_from_storage(
            &row.get::<_, String>(1).map_err(MetaStoreError::storage)?,
        )?);
    }
    Ok(reason_codes)
}

pub(super) fn resume_version_has_current_resume_candidate_classification_in_connection(
    connection: &Connection,
    resume_version_id: &ResumeVersionId,
) -> Result<bool> {
    let mut statement = connection
        .prepare(
            "SELECT classifier_epoch FROM resume_version_classification
             WHERE resume_version_id = ?1 AND status = 'resume_candidate'",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![resume_version_id.as_str()])
        .map_err(MetaStoreError::storage)?;
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        let epoch = row.get::<_, String>(0).map_err(MetaStoreError::storage)?;
        if CurrentClassifierEpoch::parse(&epoch).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn resume_version_has_resume_candidate_classification_at_epoch_in_connection(
    connection: &Connection,
    resume_version_id: &ResumeVersionId,
    classifier_epoch: &str,
) -> Result<bool> {
    if CurrentClassifierEpoch::parse(classifier_epoch).is_none() {
        return Err(MetaStoreError::invalid_value(
            "resume_version_classification.classifier_epoch",
        ));
    }
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM resume_version_classification
                WHERE resume_version_id = ?1 AND classifier_epoch = ?2
                  AND status = 'resume_candidate'
             )",
            params![resume_version_id.as_str(), classifier_epoch],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists == 1)
        .map_err(MetaStoreError::storage)
}

fn validate_source_revision_triage(triage: &SourceRevisionTriage) -> Result<()> {
    validate_epoch_and_reasons(
        &triage.triage_epoch,
        &triage.reason_codes,
        "source_revision_triage",
    )?;
    let valid = matches!(
        (triage.status, triage.reason_codes.as_slice()),
        (ClassificationStatus::OcrBacklog, [ReasonCode::OcrRequired])
            | (ClassificationStatus::Failed, [ReasonCode::ParserFailed])
    );
    if !valid {
        return Err(MetaStoreError::invalid_value("source_revision_triage"));
    }
    Ok(())
}

fn validate_resume_version_classification(
    classification: &ResumeVersionClassification,
) -> Result<()> {
    validate_epoch_and_reasons(
        &classification.classifier_epoch,
        &classification.reason_codes,
        "resume_version_classification",
    )?;
    let status_marker_valid = matches!(
        (
            classification.status,
            classification.reason_codes.as_slice()
        ),
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
            ]
        ) | (
            ClassificationStatus::Failed,
            [ReasonCode::EmptyNormalizedText | ReasonCode::ParserFailed]
        )
    );
    if !status_marker_valid
        || classification.review_disposition != review_disposition_for_status(classification.status)
    {
        return Err(MetaStoreError::invalid_value(
            "resume_version_classification",
        ));
    }
    Ok(())
}

fn validate_epoch_and_reasons(
    epoch: &str,
    reasons: &[ReasonCode],
    field: &'static str,
) -> Result<()> {
    let epoch = epoch.as_bytes();
    if epoch.is_empty()
        || epoch.len() > 64
        || !epoch
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        || reasons.is_empty()
        || reasons.len() > CLASSIFICATION_REASON_LIMIT
        || reasons
            .iter()
            .enumerate()
            .any(|(index, reason)| reasons[..index].contains(reason))
    {
        return Err(MetaStoreError::invalid_value(field));
    }
    Ok(())
}

fn review_disposition_for_status(status: ClassificationStatus) -> ReviewDisposition {
    match status {
        ClassificationStatus::NeedsReview => ReviewDisposition::Pending,
        ClassificationStatus::ResumeCandidate
        | ClassificationStatus::NonResume
        | ClassificationStatus::Failed => ReviewDisposition::NotRequired,
        ClassificationStatus::OcrBacklog => ReviewDisposition::NotRequired,
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
            "resume_version_classification.review_disposition",
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
        _ => Err(MetaStoreError::invalid_value("classification.status")),
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
                _ => Err(MetaStoreError::invalid_value(
                    "classification_reason.reason_code"
                )),
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
