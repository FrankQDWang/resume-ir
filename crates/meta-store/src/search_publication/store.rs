use rusqlite::{params, Connection, TransactionBehavior};

use crate::{
    document_status_to_storage, immutable_search::seal_resume_version,
    refresh_all_candidate_version_counts_in_connection, MetaStore, MetaStoreError, Result,
    UnixTimestamp,
};

use super::{
    model::{
        SearchPublicationCommit, SearchPublicationDraft, SearchPublicationFailure,
        SearchPublicationOutcome, SearchPublicationPrunePolicy, SearchPublicationRecord,
        SearchPublicationState, SearchPublicationValidation, TerminalDocumentUpdate,
    },
    persistence::{query_publications, search_publication_in_connection},
    validation::{
        publication_error, search_publication_fingerprint, u64_to_i64,
        validate_commit_against_publication, validate_commit_shape, validate_descriptors,
        validate_draft, validate_projected_document_states, vector_mode_storage,
    },
};

impl MetaStore {
    pub fn begin_search_publication(
        &self,
        draft: &SearchPublicationDraft,
    ) -> Result<SearchPublicationOutcome> {
        validate_draft(draft)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let (head, visible_epoch) = ready_head_and_epoch(&transaction)?;
        let outcome = if head.as_deref() == draft.base_generation.as_deref()
            && visible_epoch == draft.expected_visible_epoch
        {
            SearchPublicationOutcome::Applied
        } else {
            SearchPublicationOutcome::Superseded
        };
        transaction
            .execute(
                "INSERT INTO search_publication_journal (
                    generation, base_generation, expected_visible_epoch,
                    classifier_epoch, projection_digest, state,
                    created_at_seconds, updated_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    draft.generation,
                    draft.base_generation,
                    u64_to_i64(draft.expected_visible_epoch)?,
                    draft.classifier_epoch,
                    draft.projection_digest.as_str(),
                    if outcome == SearchPublicationOutcome::Applied {
                        SearchPublicationState::Preparing.as_str()
                    } else {
                        SearchPublicationState::Abandoned.as_str()
                    },
                    draft.now.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    pub fn validate_search_publication(
        &self,
        validation: &SearchPublicationValidation<'_>,
    ) -> Result<()> {
        validate_descriptors(validation)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let publication = search_publication_in_connection(&transaction, validation.generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        if publication.state != SearchPublicationState::Preparing
            || publication.projection_digest != *validation.fulltext.projection_digest()
        {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let publication_fingerprint = search_publication_fingerprint(
            &publication.classifier_epoch,
            &publication.projection_digest,
            validation.fulltext,
            validation.vector,
        );
        let (vector_mode, model_id, dimension) = vector_mode_storage(validation.vector.mode());
        let changed = transaction
            .execute(
                "UPDATE search_publication_journal
                 SET state = 'validated',
                     publication_fingerprint = ?21,
                     fulltext_generation = ?1,
                     fulltext_manifest_schema = ?2,
                     fulltext_index_schema = ?3,
                     fulltext_document_count = ?4,
                     fulltext_projection_digest = ?5,
                     fulltext_logical_content_digest = ?6,
                     vector_generation = ?7,
                     vector_manifest_schema = ?8,
                     vector_index_schema = ?9,
                     vector_mode = ?10,
                     vector_model_id = ?11,
                     vector_dimension = ?12,
                     vector_projection_count = ?13,
                     vector_coverage_digest = ?14,
                     vector_count = ?15,
                     vector_document_count = ?16,
                     vector_resume_version_count = ?17,
                     vector_projection_digest = ?18,
                     vector_logical_content_digest = ?19,
                     updated_at_seconds = ?20
                 WHERE generation = ?1 AND state = 'preparing'",
                params![
                    validation.generation,
                    validation.fulltext.manifest_schema(),
                    validation.fulltext.index_schema(),
                    u64_to_i64(validation.fulltext.document_count())?,
                    validation.fulltext.projection_digest().as_str(),
                    validation.fulltext.logical_content_digest().as_str(),
                    validation.vector.generation(),
                    validation.vector.manifest_schema(),
                    validation.vector.index_schema(),
                    vector_mode,
                    model_id,
                    dimension.map(i64::from),
                    u64_to_i64(validation.vector.projection_count())?,
                    validation.vector.coverage_digest().as_str(),
                    u64_to_i64(validation.vector.vector_count())?,
                    u64_to_i64(validation.vector.document_count())?,
                    u64_to_i64(validation.vector.resume_version_count())?,
                    validation.vector.projection_digest().as_str(),
                    validation.vector.logical_content_digest().as_str(),
                    validation.now.as_unix_seconds(),
                    publication_fingerprint.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn commit_search_publication(
        &self,
        commit: &SearchPublicationCommit<'_>,
    ) -> Result<SearchPublicationOutcome> {
        validate_commit_shape(commit)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let publication = search_publication_in_connection(&transaction, commit.generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        if publication.state != SearchPublicationState::Validated {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let (head, visible_epoch) = ready_head_and_epoch(&transaction)?;
        if head != publication.base_generation
            || visible_epoch != publication.expected_visible_epoch
        {
            abandon_validated(&transaction, commit.generation, commit.now)?;
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(SearchPublicationOutcome::Superseded);
        }
        validate_commit_against_publication(&transaction, commit, &publication)?;

        for projection in commit.projections {
            seal_resume_version(&transaction, &projection.resume_version_id, commit.now)?;
        }
        apply_terminal_document_updates(&transaction, commit.terminal_documents, commit.now)?;
        validate_projected_document_states(&transaction, commit.projections)?;

        transaction
            .execute(
                "INSERT INTO search_publication_commit_guard (state_key, generation)
                 VALUES ('default', ?1)",
                params![commit.generation],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute("DELETE FROM active_search_projection", [])
            .map_err(MetaStoreError::storage)?;
        for projection in commit.projections {
            transaction
                .execute(
                    "INSERT INTO active_search_projection (
                        document_id, resume_version_id, generation
                     ) VALUES (?1, ?2, ?3)",
                    params![
                        projection.document_id.as_str(),
                        projection.resume_version_id.as_str(),
                        commit.generation,
                    ],
                )
                .map_err(MetaStoreError::storage)?;
        }
        refresh_all_candidate_version_counts_in_connection(&transaction)?;
        let changed = transaction
            .execute(
                "UPDATE search_publication_journal
                 SET state = 'ready', updated_at_seconds = ?1
                 WHERE generation = ?2 AND state = 'validated'",
                params![commit.now.as_unix_seconds(), commit.generation],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let next_epoch = publication
            .expected_visible_epoch
            .checked_add(1)
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        let changed = transaction
            .execute(
                "UPDATE search_projection_state
                 SET service_state = 'ready', generation = ?1, visible_epoch = ?2,
                     repair_reason = NULL, updated_at_seconds = ?3
                 WHERE state_key = 'default' AND generation IS ?4 AND visible_epoch = ?5",
                params![
                    commit.generation,
                    u64_to_i64(next_epoch)?,
                    commit.now.as_unix_seconds(),
                    publication.base_generation,
                    u64_to_i64(publication.expected_visible_epoch)?,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let cleared_guard = transaction
            .execute(
                "DELETE FROM search_publication_commit_guard
                 WHERE state_key = 'default' AND generation = ?1",
                params![commit.generation],
            )
            .map_err(MetaStoreError::storage)?;
        if cleared_guard != 1 {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(SearchPublicationOutcome::Applied)
    }

    pub fn search_publication(&self, generation: &str) -> Result<Option<SearchPublicationRecord>> {
        search_publication_in_connection(&self.connection.borrow(), generation)
    }

    pub fn abandon_search_publication(&self, generation: &str, now: UnixTimestamp) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let publication = search_publication_in_connection(&transaction, generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        match publication.state {
            SearchPublicationState::Preparing | SearchPublicationState::Validated => {
                let changed = transaction
                    .execute(
                        "UPDATE search_publication_journal
                         SET state = 'abandoned', updated_at_seconds = ?1
                         WHERE generation = ?2 AND state = ?3",
                        params![
                            now.as_unix_seconds(),
                            generation,
                            publication.state.as_str(),
                        ],
                    )
                    .map_err(MetaStoreError::storage)?;
                if changed != 1 {
                    return Err(publication_error(SearchPublicationFailure::InvalidState));
                }
            }
            SearchPublicationState::Abandoned => {}
            SearchPublicationState::Ready => {
                return Err(publication_error(SearchPublicationFailure::InvalidState));
            }
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn recent_ready_search_publications(
        &self,
        limit: usize,
    ) -> Result<Vec<SearchPublicationRecord>> {
        let limit = i64::try_from(limit)
            .ok()
            .filter(|limit| (1..=256).contains(limit))
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
        query_publications(
            &self.connection.borrow(),
            "WHERE state = 'ready'
             ORDER BY updated_at_seconds DESC, generation DESC LIMIT ?1",
            params![limit],
        )
    }

    pub fn interrupted_search_publications(
        &self,
        limit: usize,
    ) -> Result<Vec<SearchPublicationRecord>> {
        let limit = i64::try_from(limit)
            .ok()
            .filter(|limit| (1..=256).contains(limit))
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
        query_publications(
            &self.connection.borrow(),
            "WHERE state IN ('preparing', 'validated')
             ORDER BY updated_at_seconds, generation LIMIT ?1",
            params![limit],
        )
    }

    pub fn prune_search_publication_history(
        &self,
        policy: SearchPublicationPrunePolicy,
    ) -> Result<usize> {
        let retain_ready = i64::try_from(policy.retain_ready)
            .ok()
            .filter(|value| (1..=256).contains(value))
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
        let max_delete = i64::try_from(policy.max_delete)
            .ok()
            .filter(|value| (1..=256).contains(value))
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDescriptor))?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let changed = transaction
            .execute(
                "DELETE FROM search_publication_journal
                 WHERE generation IN (
                    SELECT candidate.generation
                    FROM search_publication_journal AS candidate
                    WHERE NOT EXISTS (
                        SELECT 1 FROM search_projection_state AS head
                        WHERE head.generation = candidate.generation
                    )
                      AND NOT EXISTS (
                        SELECT 1 FROM active_search_projection AS projection
                        WHERE projection.generation = candidate.generation
                    )
                      AND (
                        (candidate.state = 'abandoned'
                         AND candidate.updated_at_seconds <= ?1)
                        OR (
                            candidate.state = 'ready'
                            AND candidate.generation NOT IN (
                                SELECT retained.generation
                                FROM search_publication_journal AS retained
                                WHERE retained.state = 'ready'
                                ORDER BY retained.updated_at_seconds DESC,
                                         retained.generation DESC
                                LIMIT ?2
                            )
                        )
                      )
                    ORDER BY candidate.updated_at_seconds, candidate.generation
                    LIMIT ?3
                 )",
                params![
                    policy.abandoned_updated_before.as_unix_seconds(),
                    retain_ready,
                    max_delete,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(changed)
    }
}

fn apply_terminal_document_updates(
    connection: &Connection,
    updates: &[TerminalDocumentUpdate],
    now: UnixTimestamp,
) -> Result<()> {
    for update in updates {
        let changed = connection
            .execute(
                "UPDATE document
                 SET status = ?1, is_deleted = ?2, updated_at_seconds = ?3
                 WHERE id = ?4 AND status = ?5 AND is_deleted = ?6
                   AND content_hash = ?7",
                params![
                    document_status_to_storage(update.terminal_status),
                    i64::from(update.terminal_is_deleted),
                    now.as_unix_seconds(),
                    update.document_id.as_str(),
                    document_status_to_storage(update.expected_status),
                    i64::from(update.expected_is_deleted),
                    update.expected_content_hash.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        }
    }
    Ok(())
}

fn abandon_validated(connection: &Connection, generation: &str, now: UnixTimestamp) -> Result<()> {
    let changed = connection
        .execute(
            "UPDATE search_publication_journal
             SET state = 'abandoned', updated_at_seconds = ?1
             WHERE generation = ?2 AND state = 'validated'",
            params![now.as_unix_seconds(), generation],
        )
        .map_err(MetaStoreError::storage)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(publication_error(SearchPublicationFailure::InvalidState))
    }
}

fn ready_head_and_epoch(connection: &Connection) -> Result<(Option<String>, u64)> {
    connection
        .query_row(
            "SELECT generation, visible_epoch FROM search_projection_state
             WHERE state_key = 'default'",
            [],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(MetaStoreError::storage)
        .and_then(|(generation, epoch)| {
            Ok((
                generation,
                u64::try_from(epoch).map_err(|_| {
                    publication_error(SearchPublicationFailure::InvalidPersistedState)
                })?,
            ))
        })
}
