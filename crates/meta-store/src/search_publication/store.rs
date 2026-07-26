use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[cfg(any(test, feature = "migration-test-support"))]
use crate::migration_rebuild_barrier::migration_rebuild_barrier_token_matches;

use crate::{
    discard_superseded_ocr_claim_in_connection, document_status_to_storage,
    file_extension_to_storage,
    immutable_search::seal_resume_version,
    ocr_claim_is_current_in_connection,
    ocr_publication::{
        complete_ocr_search_publication_claim_in_connection,
        insert_ocr_search_publication_facts_in_connection, validate_ocr_search_publication_commit,
    },
    read_document, refresh_all_candidate_version_counts_in_connection, Document, DocumentStatus,
    MetaStoreError, MetadataStore, MetadataStoreAccess, MigrationRebuildBarrierToken,
    OcrSearchPublicationCommit, OcrSearchPublicationOutcome, Result, SearchPublicationSession,
    UnixTimestamp, DOCUMENT_COLUMNS,
};

use super::{
    authority::{authority_for_begin, authority_matches_commit, PublicationAuthority},
    model::{
        ProjectedDocumentSnapshot, SearchPublicationCommit, SearchPublicationDraft,
        SearchPublicationFailure, SearchPublicationOutcome, SearchPublicationPrunePolicy,
        SearchPublicationRecord, SearchPublicationState, SearchPublicationValidation,
        TerminalDocumentUpdate,
    },
    persistence::{query_publications, search_publication_in_connection},
    retirement::{
        begin_retirement_in_connection, ensure_no_pending_retirement,
        SearchPublicationRetirementPlan,
    },
    validation::{
        publication_error, search_publication_fingerprint, u64_to_i64,
        validate_commit_against_publication, validate_commit_shape, validate_descriptors,
        validate_draft, vector_mode_storage,
    },
};

impl SearchPublicationSession {
    pub fn begin_search_publication(
        &self,
        draft: &SearchPublicationDraft,
    ) -> Result<SearchPublicationOutcome> {
        validate_draft(draft)?;
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        ensure_no_pending_retirement(&transaction)?;
        let authority = authority_for_begin(&transaction, self.active_attempt_id(), draft)?;
        let outcome = if authority.is_some() {
            SearchPublicationOutcome::Applied
        } else {
            SearchPublicationOutcome::Superseded
        };
        let authority = authority.unwrap_or(PublicationAuthority::CurrentHead);
        let authority_storage = authority.storage()?;
        transaction
            .execute(
                "INSERT INTO search_publication_journal (
                    generation, base_generation, expected_visible_epoch,
                    classifier_epoch, projection_digest, state,
                    created_at_seconds, updated_at_seconds,
                    authority_kind, authority_contract_id, authority_barrier_digest,
                    authority_repair_generation, authority_repair_fingerprint,
                    authority_repair_visible_epoch, authority_attempt_id,
                    authority_attempt_count
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, 'preparing', ?6, ?6,
                    ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                 )",
                params![
                    draft.generation,
                    draft.base_generation,
                    u64_to_i64(draft.expected_visible_epoch)?,
                    draft.classifier_epoch,
                    draft.projection_digest.as_str(),
                    draft.now.as_unix_seconds(),
                    authority_storage.kind,
                    authority_storage.contract_id,
                    authority_storage.barrier_digest,
                    authority_storage.repair_generation,
                    authority_storage.repair_fingerprint,
                    authority_storage.repair_visible_epoch,
                    authority_storage.attempt_id,
                    authority_storage.attempt_count,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if outcome == SearchPublicationOutcome::Superseded {
            begin_retirement_in_connection(
                &transaction,
                &draft.generation,
                draft.now,
                SearchPublicationRetirementPlan::none(),
            )?;
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }

    /// Synthetic v28 fixture seam used only to exercise the v28-to-v29 COW
    /// migration. Runtime v29 publication never enters this contract.
    #[cfg(feature = "migration-test-support")]
    pub(crate) fn begin_legacy_v28_search_publication_for_test(
        &self,
        draft: &SearchPublicationDraft,
    ) -> Result<SearchPublicationOutcome> {
        validate_draft(draft)?;
        if crate::schema_version_in_connection(&self.owned_store().connection.borrow())?
            != crate::schema_v28::VERSION
        {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let (head, visible_epoch) = ready_head_and_epoch(&transaction)?;
        if head != draft.base_generation || visible_epoch != draft.expected_visible_epoch {
            return Ok(SearchPublicationOutcome::Superseded);
        }
        transaction
            .execute(
                "INSERT INTO search_publication_journal (
                    generation, base_generation, expected_visible_epoch,
                    classifier_epoch, projection_digest, state,
                    created_at_seconds, updated_at_seconds
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'preparing', ?6, ?6)",
                params![
                    draft.generation,
                    draft.base_generation,
                    u64_to_i64(draft.expected_visible_epoch)?,
                    draft.classifier_epoch,
                    draft.projection_digest.as_str(),
                    draft.now.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(SearchPublicationOutcome::Applied)
    }

    pub fn validate_search_publication(
        &self,
        validation: &SearchPublicationValidation<'_>,
    ) -> Result<()> {
        validate_descriptors(validation)?;
        let mut connection = self.owned_store().connection.borrow_mut();
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
        self.commit_search_publication_with_precondition(
            commit,
            PublicationCommitPrecondition::CurrentHead,
        )
    }

    /// Commits the first v27 publication only if the all-root rebuild barrier
    /// captured before snapshot construction is still exact.
    pub fn commit_migration_rebuild_search_publication(
        &self,
        commit: &SearchPublicationCommit<'_>,
        barrier: &MigrationRebuildBarrierToken,
    ) -> Result<SearchPublicationOutcome> {
        validate_commit_shape(commit)?;
        self.commit_search_publication_with_precondition(
            commit,
            PublicationCommitPrecondition::MigrationRebuild(barrier),
        )
    }

    fn commit_search_publication_with_precondition(
        &self,
        commit: &SearchPublicationCommit<'_>,
        precondition: PublicationCommitPrecondition<'_>,
    ) -> Result<SearchPublicationOutcome> {
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let Some((publication, service_precondition)) =
            validated_publication_for_commit(&transaction, commit, precondition)?
        else {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(SearchPublicationOutcome::Superseded);
        };
        apply_search_publication_commit(&transaction, commit, &publication, &service_precondition)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(SearchPublicationOutcome::Applied)
    }

    /// Atomically publishes OCR-derived facts and the validated search
    /// generation for one exact running OCR claim.
    pub fn commit_ocr_search_publication(
        &self,
        publication: &OcrSearchPublicationCommit<'_>,
    ) -> Result<OcrSearchPublicationOutcome> {
        validate_ocr_search_publication_commit(publication)?;
        validate_commit_shape(&publication.search)?;
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        if !ocr_claim_is_current_in_connection(&transaction, publication.claimed)? {
            discard_superseded_ocr_claim_in_connection(
                &transaction,
                publication.claimed,
                publication.search.now,
            )?;
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(OcrSearchPublicationOutcome::ClaimSuperseded);
        }
        let Some((search_publication, service_precondition)) = validated_publication_for_commit(
            &transaction,
            &publication.search,
            PublicationCommitPrecondition::CurrentHead,
        )?
        else {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(OcrSearchPublicationOutcome::PublicationSuperseded);
        };
        let commit_result = (|| {
            if search_publication.classifier_epoch != publication.classification.classifier_epoch {
                return Err(publication_error(
                    SearchPublicationFailure::ExactClassificationMissing,
                ));
            }
            insert_ocr_search_publication_facts_in_connection(&transaction, publication)?;
            apply_search_publication_commit(
                &transaction,
                &publication.search,
                &search_publication,
                &service_precondition,
            )?;
            complete_ocr_search_publication_claim_in_connection(&transaction, publication)
        })();
        if let Err(error) = commit_result {
            transaction.rollback().map_err(MetaStoreError::storage)?;
            return Err(error);
        }
        if let Err(error) = transaction.commit() {
            return Err(MetaStoreError::storage(error));
        }
        Ok(OcrSearchPublicationOutcome::Applied)
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn search_publication(&self, generation: &str) -> Result<Option<SearchPublicationRecord>> {
        search_publication_in_connection(&self.connection.borrow(), generation)
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
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
}

impl SearchPublicationSession {
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
        let mut connection = self.owned_store().connection.borrow_mut();
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
                      AND NOT EXISTS (
                        SELECT 1 FROM search_publication_retirement AS retirement
                        WHERE retirement.generation = candidate.generation
                          AND retirement.phase = 'pending'
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

fn validated_publication_for_commit(
    connection: &Connection,
    commit: &SearchPublicationCommit<'_>,
    precondition: PublicationCommitPrecondition<'_>,
) -> Result<Option<(SearchPublicationRecord, PublicationServicePrecondition)>> {
    let publication = search_publication_in_connection(connection, commit.generation)?
        .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
    if publication.state != SearchPublicationState::Validated {
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    let service_precondition = publication_service_precondition(connection)?;
    let migration_barrier = match precondition {
        PublicationCommitPrecondition::CurrentHead => None,
        PublicationCommitPrecondition::MigrationRebuild(barrier) => Some(barrier),
    };
    #[cfg(any(test, feature = "migration-test-support"))]
    let precondition_matches =
        if crate::schema_version_in_connection(connection)? == crate::schema_v28::VERSION {
            migration_barrier
                .map(|barrier| migration_rebuild_barrier_token_matches(connection, barrier))
                .transpose()?
                .unwrap_or(false)
        } else {
            authority_matches_commit(connection, commit.generation, migration_barrier)?
        };
    #[cfg(not(any(test, feature = "migration-test-support")))]
    let precondition_matches =
        authority_matches_commit(connection, commit.generation, migration_barrier)?;
    let (head, visible_epoch) = ready_head_and_epoch(connection)?;
    if !precondition_matches
        || head != publication.base_generation
        || visible_epoch != publication.expected_visible_epoch
    {
        return Ok(None);
    }
    Ok(Some((publication, service_precondition)))
}

fn apply_search_publication_commit(
    connection: &Connection,
    commit: &SearchPublicationCommit<'_>,
    publication: &SearchPublicationRecord,
    service_precondition: &PublicationServicePrecondition,
) -> Result<()> {
    validate_commit_against_publication(connection, commit, publication)?;
    for projection in commit.projections {
        seal_resume_version(connection, &projection.resume_version_id, commit.now)?;
    }
    apply_terminal_document_updates(connection, commit.terminal_documents, commit.now)?;
    let projected_documents = projected_document_snapshots(connection, commit.projected_documents)?;

    connection
        .execute(
            "INSERT INTO search_publication_commit_guard (state_key, generation)
             VALUES ('default', ?1)",
            params![commit.generation],
        )
        .map_err(MetaStoreError::storage)?;
    connection
        .execute("DELETE FROM active_search_projection", [])
        .map_err(MetaStoreError::storage)?;
    for (projection, document) in commit.projections.iter().zip(&projected_documents) {
        let content_hash = document
            .content_hash
            .as_deref()
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDocumentState))?;
        connection
            .execute(
                "INSERT INTO active_search_projection (
                    document_id, resume_version_id, generation,
                    source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, content_hash, text_hash, is_deleted,
                    created_at_seconds, updated_at_seconds, status
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15
                 )",
                params![
                    projection.document_id.as_str(),
                    projection.resume_version_id.as_str(),
                    commit.generation,
                    document.source_uri,
                    document.normalized_path,
                    document.file_name,
                    file_extension_to_storage(&document.extension),
                    i64::try_from(document.byte_size).map_err(|_| {
                        publication_error(SearchPublicationFailure::InvalidDocumentState)
                    })?,
                    document.mtime.as_unix_seconds(),
                    content_hash,
                    document.text_hash,
                    i64::from(document.is_deleted),
                    document.created_at.as_unix_seconds(),
                    document.updated_at.as_unix_seconds(),
                    document_status_to_storage(document.status),
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }
    refresh_all_candidate_version_counts_in_connection(connection)?;
    let changed = connection
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
    let changed = connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'ready', generation = ?1, visible_epoch = ?2,
                 repair_reason = NULL, updated_at_seconds = ?3
             WHERE state_key = 'default' AND generation IS ?4 AND visible_epoch = ?5
               AND service_state = ?6 AND repair_reason IS ?7",
            params![
                commit.generation,
                u64_to_i64(next_epoch)?,
                commit.now.as_unix_seconds(),
                publication.base_generation,
                u64_to_i64(publication.expected_visible_epoch)?,
                service_precondition.service_state,
                service_precondition.repair_reason,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    let cleared_guard = connection
        .execute(
            "DELETE FROM search_publication_commit_guard
             WHERE state_key = 'default' AND generation = ?1",
            params![commit.generation],
        )
        .map_err(MetaStoreError::storage)?;
    if cleared_guard != 1 {
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    Ok(())
}

fn projected_document_snapshots(
    connection: &Connection,
    actions: &[ProjectedDocumentSnapshot],
) -> Result<Vec<Document>> {
    let active_sql = "SELECT document_id, source_uri, normalized_path, file_name, extension,
                byte_size, mtime_seconds, content_hash, text_hash, is_deleted,
                created_at_seconds, updated_at_seconds, status
         FROM active_search_projection
         WHERE document_id = ?1 AND resume_version_id = ?2";
    let current_sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
    let mut snapshots = Vec::with_capacity(actions.len());
    for action in actions {
        let projection = action.projection();
        let retained = connection
            .query_row(
                active_sql,
                params![
                    projection.document_id.as_str(),
                    projection.resume_version_id.as_str(),
                ],
                |row| read_document(row).map_err(|_| rusqlite::Error::InvalidQuery),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        let document = match action {
            ProjectedDocumentSnapshot::RetainedUnchanged { .. } => retained
                .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidDocumentState))?,
            ProjectedDocumentSnapshot::MetadataChanged { document, .. } => {
                let retained = retained.ok_or_else(|| {
                    publication_error(SearchPublicationFailure::InvalidDocumentState)
                })?;
                if &retained == document {
                    return Err(publication_error(
                        SearchPublicationFailure::InvalidDocumentState,
                    ));
                }
                validate_current_document_matches_snapshot(connection, &current_sql, document)?;
                document.clone()
            }
            ProjectedDocumentSnapshot::Replacement { document, .. } => {
                if retained.is_some() {
                    return Err(publication_error(
                        SearchPublicationFailure::InvalidDocumentState,
                    ));
                }
                validate_current_document_matches_snapshot(connection, &current_sql, document)?;
                document.clone()
            }
        };
        if document.id != projection.document_id
            || document.is_deleted
            || document.status != DocumentStatus::Searchable
            || !document_matches_exact_source_revision(connection, projection, &document)?
        {
            return Err(publication_error(
                SearchPublicationFailure::InvalidDocumentState,
            ));
        }
        snapshots.push(document);
    }
    Ok(snapshots)
}

fn validate_current_document_matches_snapshot(
    connection: &Connection,
    current_sql: &str,
    planned: &Document,
) -> Result<()> {
    let current = connection
        .query_row(current_sql, params![planned.id.as_str()], |row| {
            read_document(row).map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .optional()
        .map_err(MetaStoreError::storage)?;
    if current.as_ref() == Some(planned) {
        Ok(())
    } else {
        Err(publication_error(
            SearchPublicationFailure::InvalidDocumentState,
        ))
    }
}

fn document_matches_exact_source_revision(
    connection: &Connection,
    projection: &crate::ActiveSearchProjection,
    document: &Document,
) -> Result<bool> {
    let Some(content_hash) = document.content_hash.as_deref() else {
        return Ok(false);
    };
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM resume_version AS version
                JOIN source_revision AS revision
                  ON revision.id = version.source_revision_id
                 AND revision.document_id = version.document_id
                WHERE version.id = ?1 AND version.document_id = ?2
                  AND revision.content_hash = ?3 AND revision.byte_size = ?4
             )",
            params![
                projection.resume_version_id.as_str(),
                projection.document_id.as_str(),
                content_hash,
                i64::try_from(document.byte_size).map_err(|_| {
                    publication_error(SearchPublicationFailure::InvalidDocumentState)
                })?,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|matches| matches != 0)
        .map_err(MetaStoreError::storage)
}

#[derive(Clone, Copy)]
enum PublicationCommitPrecondition<'a> {
    CurrentHead,
    MigrationRebuild(&'a MigrationRebuildBarrierToken),
}

struct PublicationServicePrecondition {
    service_state: String,
    repair_reason: Option<String>,
}

fn publication_service_precondition(
    connection: &Connection,
) -> Result<PublicationServicePrecondition> {
    connection
        .query_row(
            "SELECT service_state, repair_reason
             FROM search_projection_state
             WHERE state_key = 'default'",
            [],
            |row| {
                Ok(PublicationServicePrecondition {
                    service_state: row.get(0)?,
                    repair_reason: row.get(1)?,
                })
            },
        )
        .map_err(MetaStoreError::storage)
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
