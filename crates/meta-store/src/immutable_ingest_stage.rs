use rusqlite::{Connection, TransactionBehavior};

use crate::{
    assign_candidate_from_hashed_contacts_in_connection,
    classification::{
        insert_resume_version_classification_in_connection,
        insert_source_revision_triage_in_connection, resume_version_classification_from_connection,
        source_revision_triage_from_connection,
    },
    immutable_search::{
        insert_entity_mentions_in_connection, insert_resume_version_in_connection,
        insert_source_revision_in_connection,
    },
    upsert_document_in_connection, ContactHash, Document, EntityMention, MetaStoreError,
    OwnedMetaStore, Result, ResumeVersion, ResumeVersionClassification, SourceRevision,
    SourceRevisionTriage, UnixTimestamp,
};

/// One complete immutable ingest stage that remains query-invisible until a
/// later search publication selects its exact source or resume version.
pub enum ImmutableIngestStage<'a> {
    SourceTriage {
        document: &'a Document,
        source_revision: &'a SourceRevision,
        triage: &'a SourceRevisionTriage,
    },
    ClassifiedResume {
        document: &'a Document,
        source_revision: &'a SourceRevision,
        version: &'a ResumeVersion,
        classification: &'a ResumeVersionClassification,
        mentions: &'a [EntityMention],
        email_hash: Option<&'a ContactHash>,
        phone_hash: Option<&'a ContactHash>,
    },
}

impl OwnedMetaStore {
    /// Atomically stages one immutable ingest decision before search
    /// publication. Any validation, identity, or storage failure leaves none
    /// of the stage's rows behind.
    pub fn stage_immutable_ingest(&self, stage: ImmutableIngestStage<'_>) -> Result<()> {
        validate_stage_relationships(&stage)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        match stage {
            ImmutableIngestStage::SourceTriage {
                document,
                source_revision,
                triage,
            } => {
                upsert_document_in_connection(&transaction, document)?;
                insert_source_revision_in_connection(&transaction, source_revision)?;
                insert_source_triage_decision_in_connection(&transaction, triage)?;
            }
            ImmutableIngestStage::ClassifiedResume {
                document,
                source_revision,
                version,
                classification,
                mentions,
                email_hash,
                phone_hash,
            } => {
                upsert_document_in_connection(&transaction, document)?;
                insert_source_revision_in_connection(&transaction, source_revision)?;
                insert_resume_version_in_connection(&transaction, version)?;
                insert_version_classification_decision_in_connection(&transaction, classification)?;
                insert_entity_mentions_in_connection(&transaction, &version.id, mentions)?;
                assign_candidate_from_hashed_contacts_in_connection(
                    &transaction,
                    &version.id,
                    email_hash,
                    phone_hash,
                    UnixTimestamp::from_unix_seconds(0),
                )?;
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)
    }
}

fn validate_stage_relationships(stage: &ImmutableIngestStage<'_>) -> Result<()> {
    let valid = match stage {
        ImmutableIngestStage::SourceTriage {
            document,
            source_revision,
            triage,
        } => {
            source_revision.document_id == document.id
                && triage.source_revision_id == source_revision.id
                && document.content_hash.as_deref() == Some(source_revision.content_hash.as_str())
                && document.byte_size == source_revision.byte_size
        }
        ImmutableIngestStage::ClassifiedResume {
            document,
            source_revision,
            version,
            classification,
            ..
        } => {
            source_revision.document_id == document.id
                && version.document_id == document.id
                && version.source_revision_id == source_revision.id
                && classification.resume_version_id == version.id
                && document.content_hash.as_deref() == Some(source_revision.content_hash.as_str())
                && document.byte_size == source_revision.byte_size
        }
    };
    if valid {
        Ok(())
    } else {
        Err(MetaStoreError::invalid_value(
            "immutable_ingest_stage.identity",
        ))
    }
}

fn insert_source_triage_decision_in_connection(
    connection: &Connection,
    triage: &SourceRevisionTriage,
) -> Result<()> {
    match source_revision_triage_from_connection(
        connection,
        &triage.source_revision_id,
        &triage.triage_epoch,
    )? {
        Some(existing) if same_source_triage_decision(&existing, triage) => Ok(()),
        _ => insert_source_revision_triage_in_connection(connection, triage).map(|_| ()),
    }
}

fn insert_version_classification_decision_in_connection(
    connection: &Connection,
    classification: &ResumeVersionClassification,
) -> Result<()> {
    match resume_version_classification_from_connection(
        connection,
        &classification.resume_version_id,
        &classification.classifier_epoch,
    )? {
        Some(existing) if same_version_classification_decision(&existing, classification) => Ok(()),
        _ => insert_resume_version_classification_in_connection(connection, classification)
            .map(|_| ()),
    }
}

fn same_source_triage_decision(
    existing: &SourceRevisionTriage,
    proposed: &SourceRevisionTriage,
) -> bool {
    existing.source_revision_id == proposed.source_revision_id
        && existing.status == proposed.status
        && existing.triage_epoch == proposed.triage_epoch
        && existing.reason_codes == proposed.reason_codes
}

fn same_version_classification_decision(
    existing: &ResumeVersionClassification,
    proposed: &ResumeVersionClassification,
) -> bool {
    existing.resume_version_id == proposed.resume_version_id
        && existing.status == proposed.status
        && existing.classifier_epoch == proposed.classifier_epoch
        && existing.reason_codes == proposed.reason_codes
        && existing.review_disposition == proposed.review_disposition
}

#[cfg(test)]
#[path = "immutable_ingest_stage_tests.rs"]
mod tests;
