use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{
    migration_rebuild_barrier::migration_rebuild_barrier_digest_matches, MetaStoreError,
    MetadataStore, MetadataStoreAccess, Result, SearchPublicationSession, UnixTimestamp,
};

use super::{
    authority::{
        exact_blocked_current_head, exact_running_artifact_attempt,
        exact_running_migration_attempt, exact_terminal_artifact_cleanup_authority,
        exact_terminal_migration_cleanup_authority, read_authority, PublicationAuthority,
    },
    model::{
        SearchPublicationFailure, SearchPublicationRetirementFailureOutcome, SearchPublicationState,
    },
    persistence::search_publication_in_connection,
    retirement_settlement::{
        block_artifact_head, block_current_head, block_migration_head, settle_artifact_attempt,
        settle_migration_attempt,
    },
    validation::publication_error,
};

pub const SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchArtifactExpectation {
    None,
    MayExist,
    Published,
}

impl SearchArtifactExpectation {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MayExist => "may_exist",
            Self::Published => "published",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "may_exist" => Ok(Self::MayExist),
            "published" => Ok(Self::Published),
            _ => Err(publication_error(
                SearchPublicationFailure::InvalidPersistedState,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchPublicationRetirementPlan {
    pub fulltext: SearchArtifactExpectation,
    pub vector: SearchArtifactExpectation,
}

impl SearchPublicationRetirementPlan {
    pub const fn none() -> Self {
        Self {
            fulltext: SearchArtifactExpectation::None,
            vector: SearchArtifactExpectation::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationRetirementArtifact {
    FullText,
    Vector,
}

impl SearchPublicationRetirementArtifact {
    fn as_str(self) -> &'static str {
        match self {
            Self::FullText => "fulltext",
            Self::Vector => "vector",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationRetirementPhase {
    Pending,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPublicationRetirement {
    pub generation: String,
    pub phase: SearchPublicationRetirementPhase,
    pub plan: SearchPublicationRetirementPlan,
    pub fulltext_complete: bool,
    pub vector_complete: bool,
}

impl SearchPublicationSession {
    /// Atomically records exact artifact expectations before any physical
    /// deletion may start and makes the publication query-ineligible.
    pub fn begin_search_publication_retirement(
        &self,
        generation: &str,
        now: UnixTimestamp,
        plan: SearchPublicationRetirementPlan,
    ) -> Result<()> {
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        begin_retirement_in_connection(&transaction, generation, now, plan)?;
        transaction.commit().map_err(MetaStoreError::storage)
    }

    /// Abandons a publication known to have created no physical artifact.
    pub fn abandon_search_publication(&self, generation: &str, now: UnixTimestamp) -> Result<()> {
        self.begin_search_publication_retirement(
            generation,
            now,
            SearchPublicationRetirementPlan::none(),
        )
    }

    pub fn complete_search_publication_retirement_artifact(
        &self,
        generation: &str,
        artifact: SearchPublicationRetirementArtifact,
        now: UnixTimestamp,
    ) -> Result<()> {
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let existing = retirement_in_connection(&transaction, generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        let already_complete = match artifact {
            SearchPublicationRetirementArtifact::FullText => existing.fulltext_complete,
            SearchPublicationRetirementArtifact::Vector => existing.vector_complete,
        };
        if already_complete {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(());
        }
        if existing.phase != SearchPublicationRetirementPhase::Pending {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let guard_inserted = transaction
            .execute(
                "INSERT INTO search_publication_retirement_completion_guard (
                    generation, artifact, completed_at_seconds
                 ) VALUES (?1, ?2, ?3)",
                params![generation, artifact.as_str(), now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        if guard_inserted != 1 {
            return Err(MetaStoreError::storage_invariant());
        }
        let (column, peer_column) = match artifact {
            SearchPublicationRetirementArtifact::FullText => {
                ("fulltext_complete", "vector_complete")
            }
            SearchPublicationRetirementArtifact::Vector => ("vector_complete", "fulltext_complete"),
        };
        let sql = format!(
            "UPDATE search_publication_retirement
             SET {column} = 1,
                 phase = CASE WHEN {peer_column} = 1 THEN 'complete' ELSE 'pending' END,
                 updated_at_seconds = MAX(updated_at_seconds, ?1)
             WHERE generation = ?2 AND phase = 'pending' AND {column} = 0"
        );
        let changed = transaction
            .execute(&sql, params![now.as_unix_seconds(), generation])
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let guard_cleared = transaction
            .execute(
                "DELETE FROM search_publication_retirement_completion_guard
                 WHERE generation = ?1 AND artifact = ?2",
                params![generation, artifact.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        if guard_cleared != 1 {
            return Err(MetaStoreError::storage_invariant());
        }
        transaction.commit().map_err(MetaStoreError::storage)
    }

    /// Atomically settles the exact running authority and blocks only the head
    /// it still owns. A stale cleanup cannot mutate either a replacement
    /// attempt or a superseding head.
    pub fn block_search_head_after_publication_retirement_failure(
        &self,
        generation: &str,
        now: UnixTimestamp,
    ) -> Result<SearchPublicationRetirementFailureOutcome> {
        self.settle_search_publication_retirement_failure(generation, now, false)
    }

    #[cfg(test)]
    pub(crate) fn fail_search_publication_retirement_settlement_before_commit_for_test(
        &self,
        generation: &str,
        now: UnixTimestamp,
    ) -> Result<SearchPublicationRetirementFailureOutcome> {
        self.settle_search_publication_retirement_failure(generation, now, true)
    }

    fn settle_search_publication_retirement_failure(
        &self,
        generation: &str,
        now: UnixTimestamp,
        fail_before_commit: bool,
    ) -> Result<SearchPublicationRetirementFailureOutcome> {
        let mut connection = self.owned_store().connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        if !retirement_is_pending(&transaction, generation)? {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let publication = search_publication_in_connection(&transaction, generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
        if publication.state != SearchPublicationState::Abandoned {
            return Err(publication_error(SearchPublicationFailure::InvalidState));
        }
        let authority = read_authority(&transaction, generation)?
            .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidPersistedState))?;
        let outcome = match authority {
            PublicationAuthority::CurrentHead => {
                if block_current_head(
                    &transaction,
                    publication.base_generation.as_deref(),
                    publication.expected_visible_epoch,
                    now,
                )? {
                    SearchPublicationRetirementFailureOutcome::HeadBlocked
                } else if exact_blocked_current_head(
                    &transaction,
                    publication.base_generation.as_deref(),
                    publication.expected_visible_epoch,
                )? {
                    SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
                } else {
                    SearchPublicationRetirementFailureOutcome::HeadSuperseded
                }
            }
            PublicationAuthority::MigrationRebuild {
                contract_id,
                barrier_digest,
                attempt_id,
                attempt_count,
            } => {
                if publication.base_generation.is_none()
                    && exact_blocked_current_head(
                        &transaction,
                        None,
                        publication.expected_visible_epoch,
                    )?
                    && exact_terminal_migration_cleanup_authority(
                        &transaction,
                        &contract_id,
                        &barrier_digest,
                        &attempt_id,
                        attempt_count,
                    )?
                {
                    SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
                } else if !migration_rebuild_barrier_digest_matches(
                    &transaction,
                    &contract_id,
                    &barrier_digest,
                )? || !exact_running_migration_attempt(
                    &transaction,
                    &contract_id,
                    &barrier_digest,
                    &attempt_id,
                    attempt_count,
                )? {
                    SearchPublicationRetirementFailureOutcome::HeadSuperseded
                } else {
                    let settled = settle_migration_attempt(
                        &transaction,
                        &contract_id,
                        &barrier_digest,
                        &attempt_id,
                        attempt_count,
                        now,
                    )?;
                    if !settled {
                        transaction.rollback().map_err(MetaStoreError::storage)?;
                        return Ok(SearchPublicationRetirementFailureOutcome::HeadSuperseded);
                    }
                    let blocked = block_migration_head(&transaction, &contract_id, now)?;
                    if !blocked {
                        transaction.rollback().map_err(MetaStoreError::storage)?;
                        return Ok(SearchPublicationRetirementFailureOutcome::HeadSuperseded);
                    }
                    SearchPublicationRetirementFailureOutcome::HeadBlocked
                }
            }
            PublicationAuthority::ArtifactRepair {
                key,
                attempt_id,
                attempt_count,
            } => {
                if publication.base_generation.as_deref() == Some(key.generation())
                    && publication.expected_visible_epoch == key.visible_epoch()
                    && exact_terminal_artifact_cleanup_authority(
                        &transaction,
                        &key,
                        &attempt_id,
                        attempt_count,
                    )?
                {
                    SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
                } else if !exact_running_artifact_attempt(
                    &transaction,
                    &key,
                    &attempt_id,
                    attempt_count,
                )? {
                    SearchPublicationRetirementFailureOutcome::HeadSuperseded
                } else {
                    let settled = settle_artifact_attempt(
                        &transaction,
                        &key,
                        &attempt_id,
                        attempt_count,
                        now,
                    )?;
                    if !settled {
                        transaction.rollback().map_err(MetaStoreError::storage)?;
                        return Ok(SearchPublicationRetirementFailureOutcome::HeadSuperseded);
                    }
                    let blocked = block_artifact_head(&transaction, &key, now)?;
                    if !blocked {
                        transaction.rollback().map_err(MetaStoreError::storage)?;
                        return Ok(SearchPublicationRetirementFailureOutcome::HeadSuperseded);
                    }
                    SearchPublicationRetirementFailureOutcome::HeadBlocked
                }
            }
        };
        if fail_before_commit {
            return Err(MetaStoreError::storage_invariant());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(outcome)
    }
}

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn pending_search_publication_retirements(
        &self,
    ) -> Result<Vec<SearchPublicationRetirement>> {
        pending_retirements_in_connection(&self.connection.borrow())
    }

    pub fn search_publication_retirement(
        &self,
        generation: &str,
    ) -> Result<Option<SearchPublicationRetirement>> {
        retirement_in_connection(&self.connection.borrow(), generation)
    }
}

pub(crate) fn ensure_no_pending_retirement(connection: &Connection) -> Result<()> {
    #[cfg(any(test, feature = "migration-test-support"))]
    if crate::schema_version_in_connection(connection)? < crate::schema_v29::VERSION {
        return Ok(());
    }
    if !pending_retirements_in_connection(connection)?.is_empty() {
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    Ok(())
}

pub(super) fn begin_retirement_in_connection(
    connection: &Connection,
    generation: &str,
    now: UnixTimestamp,
    plan: SearchPublicationRetirementPlan,
) -> Result<()> {
    let publication = search_publication_in_connection(connection, generation)?
        .ok_or_else(|| publication_error(SearchPublicationFailure::InvalidState))?;
    if publication.state == SearchPublicationState::Ready {
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    if let Some(existing) = retirement_in_connection(connection, generation)? {
        if existing.plan == plan && publication.state == SearchPublicationState::Abandoned {
            return Ok(());
        }
        return Err(publication_error(SearchPublicationFailure::InvalidState));
    }
    let fulltext_complete = i64::from(plan.fulltext == SearchArtifactExpectation::None);
    let vector_complete = i64::from(plan.vector == SearchArtifactExpectation::None);
    let phase = if fulltext_complete == 1 && vector_complete == 1 {
        "complete"
    } else {
        "pending"
    };
    connection
        .execute(
            "INSERT INTO search_publication_retirement (
                generation, phase, fulltext_expectation, vector_expectation,
                fulltext_complete, vector_complete, created_at_seconds,
                updated_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                generation,
                phase,
                plan.fulltext.as_str(),
                plan.vector.as_str(),
                fulltext_complete,
                vector_complete,
                now.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if publication.state != SearchPublicationState::Abandoned {
        let changed = connection
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
    Ok(())
}

fn pending_retirements_in_connection(
    connection: &Connection,
) -> Result<Vec<SearchPublicationRetirement>> {
    let limit = i64::try_from(SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT + 1)
        .map_err(|_| MetaStoreError::storage_invariant())?;
    let mut statement = connection
        .prepare(
            "SELECT generation, phase, fulltext_expectation, vector_expectation,
                    fulltext_complete, vector_complete
             FROM search_publication_retirement
             WHERE phase = 'pending'
             ORDER BY updated_at_seconds, generation LIMIT ?1",
        )
        .map_err(MetaStoreError::storage)?;
    let rows = statement
        .query_map(params![limit], read_retirement_row)
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    if rows.len() > SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT {
        return Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        ));
    }
    rows.into_iter().map(parse_retirement_row).collect()
}

fn retirement_in_connection(
    connection: &Connection,
    generation: &str,
) -> Result<Option<SearchPublicationRetirement>> {
    connection
        .query_row(
            "SELECT generation, phase, fulltext_expectation, vector_expectation,
                    fulltext_complete, vector_complete
             FROM search_publication_retirement WHERE generation = ?1",
            params![generation],
            read_retirement_row,
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .map(parse_retirement_row)
        .transpose()
}

type RetirementRow = (String, String, String, String, i64, i64);

fn read_retirement_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RetirementRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn parse_retirement_row(row: RetirementRow) -> Result<SearchPublicationRetirement> {
    let phase = match row.1.as_str() {
        "pending" => SearchPublicationRetirementPhase::Pending,
        "complete" => SearchPublicationRetirementPhase::Complete,
        _ => {
            return Err(publication_error(
                SearchPublicationFailure::InvalidPersistedState,
            ))
        }
    };
    let fulltext_complete = parse_bool(row.4)?;
    let vector_complete = parse_bool(row.5)?;
    Ok(SearchPublicationRetirement {
        generation: row.0,
        phase,
        plan: SearchPublicationRetirementPlan {
            fulltext: SearchArtifactExpectation::parse(&row.2)?,
            vector: SearchArtifactExpectation::parse(&row.3)?,
        },
        fulltext_complete,
        vector_complete,
    })
}

fn parse_bool(value: i64) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(publication_error(
            SearchPublicationFailure::InvalidPersistedState,
        )),
    }
}

fn retirement_is_pending(connection: &Connection, generation: &str) -> Result<bool> {
    Ok(retirement_in_connection(connection, generation)?
        .is_some_and(|record| record.phase == SearchPublicationRetirementPhase::Pending))
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod tests;
