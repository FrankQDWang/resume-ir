use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{
    immutable_ingest_stage::stage_immutable_ingest_in_connection,
    import_task_purpose::import_task_purpose_in_connection,
    ocr_job_enqueue::enqueue_ocr_job_for_source_triage_in_connection,
    source_file_observation::record_strong_source_file_observation_in_connection,
    source_root_commit_fence::validate_import_commit, upsert_document_in_connection, Document,
    DocumentId, ImmutableIngestStage, ImportTaskId, ImportTaskPurpose, MetaStoreError,
    OccurrenceChange, OwnedMetaStore, Result, SourceRevisionId, StrongSourceFileObservation,
    UnixTimestamp,
};

pub enum SourceRootBoundImportMutation<'a> {
    Immutable(ImmutableIngestStage<'a>),
    OcrRequired(ImmutableIngestStage<'a>),
    ExistingRevision {
        document_id: &'a DocumentId,
        source_revision_id: &'a SourceRevisionId,
    },
    ExistingRevisionMetadata {
        document: &'a Document,
        source_revision_id: &'a SourceRevisionId,
    },
    ReadFailureWithoutRevision,
}

#[derive(Clone, Copy)]
pub enum SourceRootBoundImportObservation<'a> {
    MetadataOnly,
    Strong(&'a StrongSourceFileObservation),
}

pub struct SourceRootBoundImportCommit<'a> {
    pub task_id: &'a ImportTaskId,
    pub normalized_path: &'a str,
    pub observed_at: UnixTimestamp,
    pub mutation: SourceRootBoundImportMutation<'a>,
    pub observation: SourceRootBoundImportObservation<'a>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceRootBoundImportCommitOutcome {
    pub ocr_job_scheduled: bool,
}

impl OwnedMetaStore {
    pub fn commit_source_root_bound_import(
        &self,
        commit: SourceRootBoundImportCommit<'_>,
    ) -> Result<SourceRootBoundImportCommitOutcome> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        if import_task_purpose_in_connection(&transaction, commit.task_id)?
            != ImportTaskPurpose::ConfiguredCatchUp
        {
            return Err(MetaStoreError::invalid_transition());
        }
        let identity =
            validate_import_commit(&transaction, commit.task_id, commit.normalized_path)?;
        let mut outcome = SourceRootBoundImportCommitOutcome::default();
        match commit.mutation {
            SourceRootBoundImportMutation::Immutable(stage) => {
                let (document_id, source_revision_id) = stage_source(&stage);
                validate_observation_source(commit.observation, &source_revision_id)?;
                stage_immutable_ingest_in_connection(&transaction, stage)?;
                commit_source_identity(
                    &transaction,
                    &identity,
                    commit.task_id,
                    &document_id,
                    &source_revision_id,
                    commit.observed_at,
                    commit.observation,
                )?;
            }
            SourceRootBoundImportMutation::OcrRequired(stage) => {
                let (document_id, source_revision_id) = stage_source(&stage);
                validate_observation_source(commit.observation, &source_revision_id)?;
                let ImmutableIngestStage::SourceTriage { triage, .. } = &stage else {
                    return Err(MetaStoreError::invalid_value(
                        "source_root_bound_import.ocr_stage",
                    ));
                };
                let triage_epoch = triage.triage_epoch.clone();
                stage_immutable_ingest_in_connection(&transaction, stage)?;
                commit_source_identity(
                    &transaction,
                    &identity,
                    commit.task_id,
                    &document_id,
                    &source_revision_id,
                    commit.observed_at,
                    commit.observation,
                )?;
                outcome.ocr_job_scheduled = enqueue_ocr_job_for_source_triage_in_connection(
                    &transaction,
                    &source_revision_id,
                    &triage_epoch,
                    commit.observed_at,
                )?
                .1;
            }
            SourceRootBoundImportMutation::ExistingRevision {
                document_id,
                source_revision_id,
            } => {
                validate_existing_revision(&transaction, document_id, source_revision_id)?;
                validate_observation_source(commit.observation, source_revision_id)?;
                commit_source_identity(
                    &transaction,
                    &identity,
                    commit.task_id,
                    document_id,
                    source_revision_id,
                    commit.observed_at,
                    commit.observation,
                )?;
            }
            SourceRootBoundImportMutation::ExistingRevisionMetadata {
                document,
                source_revision_id,
            } => {
                validate_existing_revision(&transaction, &document.id, source_revision_id)?;
                validate_observation_source(commit.observation, source_revision_id)?;
                upsert_document_in_connection(&transaction, document)?;
                commit_source_identity(
                    &transaction,
                    &identity,
                    commit.task_id,
                    &document.id,
                    source_revision_id,
                    commit.observed_at,
                    commit.observation,
                )?;
            }
            SourceRootBoundImportMutation::ReadFailureWithoutRevision => {
                if matches!(
                    commit.observation,
                    SourceRootBoundImportObservation::Strong(_)
                ) {
                    return Err(MetaStoreError::invalid_value(
                        "source_root_bound_import.observation",
                    ));
                }
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }
}

fn stage_source(stage: &ImmutableIngestStage<'_>) -> (DocumentId, SourceRevisionId) {
    match stage {
        ImmutableIngestStage::SourceTriage {
            document,
            source_revision,
            ..
        }
        | ImmutableIngestStage::ClassifiedResume {
            document,
            source_revision,
            ..
        } => (document.id.clone(), source_revision.id.clone()),
    }
}

#[allow(clippy::too_many_arguments)]
fn commit_source_identity(
    connection: &Connection,
    identity: &crate::source_root_commit_fence::RootBoundImportIdentity,
    task_id: &ImportTaskId,
    document_id: &DocumentId,
    source_revision_id: &SourceRevisionId,
    observed_at: UnixTimestamp,
    observation: SourceRootBoundImportObservation<'_>,
) -> Result<()> {
    observe_source_occurrence_in_connection(
        connection,
        &identity.root_id,
        &identity.relative_path,
        document_id,
        source_revision_id,
        task_id.as_str(),
        observed_at,
    )?;
    if let SourceRootBoundImportObservation::Strong(observation) = observation {
        record_strong_source_file_observation_in_connection(
            connection,
            &identity.root_id,
            &identity.relative_path,
            observation,
        )?;
    }
    Ok(())
}

fn validate_observation_source(
    observation: SourceRootBoundImportObservation<'_>,
    source_revision_id: &SourceRevisionId,
) -> Result<()> {
    if let SourceRootBoundImportObservation::Strong(observation) = observation {
        if &observation.source_revision_id != source_revision_id {
            return Err(MetaStoreError::invalid_value(
                "source_root_bound_import.observation",
            ));
        }
    }
    Ok(())
}

fn validate_existing_revision(
    connection: &Connection,
    document_id: &DocumentId,
    source_revision_id: &SourceRevisionId,
) -> Result<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM source_revision
             WHERE id = ?1 AND document_id = ?2",
            params![source_revision_id.as_str(), document_id.as_str()],
            |_| Ok(()),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .is_some();
    if !exists {
        return Err(MetaStoreError::invalid_transition());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn observe_source_occurrence_in_connection(
    connection: &Connection,
    root_id: &crate::SourceRootId,
    relative_path: &str,
    document_id: &DocumentId,
    source_revision_id: &SourceRevisionId,
    scan_id: &str,
    now: UnixTimestamp,
) -> Result<OccurrenceChange> {
    super::source_roots::validate_relative_path(relative_path)?;
    let previous = connection
        .query_row(
            "SELECT document_id, source_revision_id, state
             FROM source_occurrence
             WHERE root_id = ?1 AND relative_path = ?2",
            params![root_id.as_str(), relative_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let change = match previous.as_ref() {
        None => OccurrenceChange::Inserted,
        Some((old_document, old_revision, state))
            if old_document == document_id.as_str()
                && old_revision == source_revision_id.as_str()
                && state == "present" =>
        {
            OccurrenceChange::Unchanged
        }
        Some(_) => OccurrenceChange::Replaced,
    };
    connection
        .execute(
            "INSERT INTO source_occurrence (
                root_id, relative_path, document_id, source_revision_id,
                state, first_seen_scan_id, last_seen_scan_id,
                observed_at_seconds, removed_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, 'present', ?5, ?5, ?6, NULL)
             ON CONFLICT(root_id, relative_path) DO UPDATE SET
                document_id = excluded.document_id,
                source_revision_id = excluded.source_revision_id,
                state = 'present',
                last_seen_scan_id = excluded.last_seen_scan_id,
                observed_at_seconds = excluded.observed_at_seconds,
                removed_at_seconds = NULL",
            params![
                root_id.as_str(),
                relative_path,
                document_id.as_str(),
                source_revision_id.as_str(),
                scan_id,
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO source_occurrence_revision (
                root_id, relative_path, source_revision_id, observed_at_seconds
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                root_id.as_str(),
                relative_path,
                source_revision_id.as_str(),
                now.as_unix_seconds()
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if let Some((old_document, _, _)) = previous {
        if old_document != document_id.as_str() {
            tombstone_unreferenced_document(connection, &old_document, now)?;
        }
    }
    Ok(change)
}

pub(super) fn tombstone_unreferenced_document(
    connection: &Connection,
    document_id: &str,
    now: UnixTimestamp,
) -> Result<()> {
    connection
        .execute(
            "UPDATE document
             SET is_deleted = 1, status = 'deleted',
                 updated_at_seconds = MAX(updated_at_seconds, ?2)
             WHERE id = ?1
               AND NOT EXISTS (
                    SELECT 1 FROM source_occurrence
                    WHERE document_id = ?1 AND state = 'present'
               )",
            params![document_id, now.as_unix_seconds()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

#[cfg(test)]
#[path = "source_root_bound_import_commit_tests.rs"]
mod tests;
