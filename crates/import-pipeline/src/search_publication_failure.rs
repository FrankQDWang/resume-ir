//! Exact cleanup for search publications that did not become active.

use std::collections::BTreeSet;

use index_fulltext::FullTextGenerationRetirement;
use index_vector::VectorGenerationRetirement;
use meta_store::{
    SearchArtifactExpectation, SearchPublicationRetirementArtifact,
    SearchPublicationRetirementFailureOutcome, SearchPublicationRetirementPlan,
    SearchPublicationSession, SearchPublicationState, UnixTimestamp,
};

use super::{ImportPipelineError, ImportPipelineErrorKind, Result};

const RETAIN_READY_GENERATIONS_FOR_EXACT_RETIREMENT: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FailedGenerationArtifactState {
    #[default]
    None,
    MayExist,
    Published,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FailedGenerationArtifacts {
    fulltext: FailedGenerationArtifactState,
    vector: FailedGenerationArtifactState,
}

impl FailedGenerationArtifacts {
    pub(super) fn both_may_exist() -> Self {
        Self {
            fulltext: FailedGenerationArtifactState::MayExist,
            vector: FailedGenerationArtifactState::MayExist,
        }
    }

    pub(super) fn both_published() -> Self {
        Self {
            fulltext: FailedGenerationArtifactState::Published,
            vector: FailedGenerationArtifactState::Published,
        }
    }

    pub(super) fn record_fulltext_published(&mut self) {
        self.fulltext = FailedGenerationArtifactState::Published;
    }

    pub(super) fn record_fulltext_failure(&mut self, error: &ImportPipelineError) {
        if error.kind != ImportPipelineErrorKind::FullTextPublicationBusy {
            self.fulltext = FailedGenerationArtifactState::MayExist;
        }
    }

    pub(super) fn record_vector_published(&mut self) {
        self.vector = FailedGenerationArtifactState::Published;
    }

    pub(super) fn record_vector_failure(&mut self, error: &ImportPipelineError) {
        if error.kind != ImportPipelineErrorKind::VectorPublicationBusy {
            self.vector = FailedGenerationArtifactState::MayExist;
        }
    }

    fn retirement_error(self) -> Option<ImportPipelineError> {
        if self.fulltext != FailedGenerationArtifactState::None {
            Some(ImportPipelineError::fulltext_artifact_retirement())
        } else if self.vector != FailedGenerationArtifactState::None {
            Some(ImportPipelineError::vector_artifact_retirement())
        } else {
            None
        }
    }

    fn retirement_plan(self) -> SearchPublicationRetirementPlan {
        SearchPublicationRetirementPlan {
            fulltext: expectation(self.fulltext),
            vector: expectation(self.vector),
        }
    }
}

fn expectation(state: FailedGenerationArtifactState) -> SearchArtifactExpectation {
    match state {
        FailedGenerationArtifactState::None => SearchArtifactExpectation::None,
        FailedGenerationArtifactState::MayExist => SearchArtifactExpectation::MayExist,
        FailedGenerationArtifactState::Published => SearchArtifactExpectation::Published,
    }
}

pub(super) fn abandon_and_retire_search_publication(
    publication_session: &SearchPublicationSession,
    generation: &str,
    now: UnixTimestamp,
    failed_artifacts: FailedGenerationArtifacts,
) -> Result<()> {
    if let Err(error) = publication_session.begin_search_publication_retirement(
        generation,
        now,
        failed_artifacts.retirement_plan(),
    ) {
        return Err(failed_artifacts
            .retirement_error()
            .unwrap_or_else(|| ImportPipelineError::store(error)));
    }
    retire_abandoned_search_publication_generation(publication_session, generation, now)
}

pub(super) fn retire_abandoned_search_publication_generation(
    publication_session: &SearchPublicationSession,
    generation: &str,
    now: UnixTimestamp,
) -> Result<()> {
    retire_abandoned_search_publication_generation_classified(publication_session, generation, now)
        .map_err(SearchPublicationRetirementError::into_inner)
}

enum SearchPublicationRetirementError {
    CurrentHeadBlocked(ImportPipelineError),
    Other(ImportPipelineError),
}

impl SearchPublicationRetirementError {
    fn into_inner(self) -> ImportPipelineError {
        match self {
            Self::CurrentHeadBlocked(error) | Self::Other(error) => error,
        }
    }
}

fn retire_abandoned_search_publication_generation_classified(
    publication_session: &SearchPublicationSession,
    generation: &str,
    now: UnixTimestamp,
) -> std::result::Result<(), SearchPublicationRetirementError> {
    match try_retire_abandoned_search_publication_generation(publication_session, generation, now) {
        Ok(()) => Ok(()),
        Err(retirement_error) => {
            match publication_session
                .block_search_head_after_publication_retirement_failure(generation, now)
                .map_err(|_| SearchPublicationRetirementError::Other(retirement_error.clone()))?
            {
                SearchPublicationRetirementFailureOutcome::HeadBlocked
                | SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked => {
                    return Err(SearchPublicationRetirementError::CurrentHeadBlocked(
                        retirement_error,
                    ));
                }
                SearchPublicationRetirementFailureOutcome::HeadSuperseded => {}
            }
            Err(SearchPublicationRetirementError::Other(retirement_error))
        }
    }
}

pub(super) enum PendingRetirementReplay {
    Replayed(usize),
    CurrentHeadBlocked(ImportPipelineError),
}

pub(super) fn replay_pending_search_publication_retirements_classified(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
) -> Result<PendingRetirementReplay> {
    let pending = publication_session
        .owned_store()
        .pending_search_publication_retirements()
        .map_err(ImportPipelineError::store)?;
    for retirement in &pending {
        match retire_abandoned_search_publication_generation_classified(
            publication_session,
            &retirement.generation,
            now,
        ) {
            Ok(()) => {}
            Err(SearchPublicationRetirementError::CurrentHeadBlocked(error)) => {
                return Ok(PendingRetirementReplay::CurrentHeadBlocked(error));
            }
            Err(SearchPublicationRetirementError::Other(error)) => return Err(error),
        }
    }
    if !publication_session
        .owned_store()
        .pending_search_publication_retirements()
        .map_err(ImportPipelineError::store)?
        .is_empty()
    {
        return Err(ImportPipelineError::store_invariant());
    }
    Ok(PendingRetirementReplay::Replayed(pending.len()))
}

pub(super) fn replay_pending_search_publication_retirements(
    publication_session: &SearchPublicationSession,
    now: UnixTimestamp,
) -> Result<usize> {
    match replay_pending_search_publication_retirements_classified(publication_session, now)? {
        PendingRetirementReplay::Replayed(replayed) => Ok(replayed),
        PendingRetirementReplay::CurrentHeadBlocked(error) => Err(error),
    }
}

fn try_retire_abandoned_search_publication_generation(
    publication_session: &SearchPublicationSession,
    generation: &str,
    now: UnixTimestamp,
) -> Result<()> {
    let store = publication_session.owned_store();
    let publication = store
        .search_publication(generation)
        .map_err(|_| fulltext_retirement_failure())?
        .ok_or_else(fulltext_retirement_failure)?;
    if publication.state != SearchPublicationState::Abandoned {
        return Err(fulltext_retirement_failure());
    }
    let retirement = store
        .search_publication_retirement(generation)
        .map_err(|_| fulltext_retirement_failure())?
        .ok_or_else(fulltext_retirement_failure)?;
    let retained = store
        .search_artifact_retention_generations(RETAIN_READY_GENERATIONS_FOR_EXACT_RETIREMENT)
        .map_err(|_| fulltext_retirement_failure())?;
    if retained.contains(generation) {
        return Err(fulltext_retirement_failure());
    }

    if !retirement.fulltext_complete {
        retire_failed_fulltext_generation(
            publication_session.canonical_data_dir(),
            generation,
            &retained,
            retirement.plan.fulltext,
        )?;
        publication_session
            .complete_search_publication_retirement_artifact(
                generation,
                SearchPublicationRetirementArtifact::FullText,
                now,
            )
            .map_err(|_| fulltext_retirement_failure())?;
    }
    if !retirement.vector_complete {
        retire_failed_vector_generation(
            publication_session.canonical_data_dir(),
            generation,
            &retained,
            retirement.plan.vector,
        )?;
        publication_session
            .complete_search_publication_retirement_artifact(
                generation,
                SearchPublicationRetirementArtifact::Vector,
                now,
            )
            .map_err(|_| vector_retirement_failure())?;
    }
    Ok(())
}

fn fulltext_retirement_failure() -> ImportPipelineError {
    ImportPipelineError::fulltext_artifact_retirement()
}

fn vector_retirement_failure() -> ImportPipelineError {
    ImportPipelineError::vector_artifact_retirement()
}

fn retire_failed_fulltext_generation(
    data_dir: &std::path::Path,
    generation: &str,
    retained: &BTreeSet<String>,
    expectation: SearchArtifactExpectation,
) -> Result<()> {
    if expectation == SearchArtifactExpectation::None {
        return Ok(());
    }
    let outcome = index_fulltext::try_retire_unpublished_generation(
        &data_dir.join("search-index"),
        generation,
        retained,
    )
    .map_err(|_| fulltext_retirement_failure())?;
    match outcome {
        FullTextGenerationRetirement::Deferred => Err(fulltext_retirement_failure()),
        FullTextGenerationRetirement::Retired(summary)
            if expectation == SearchArtifactExpectation::Published
                && (!summary.removed_snapshot() || !summary.removed_generation_pin()) =>
        {
            Err(fulltext_retirement_failure())
        }
        FullTextGenerationRetirement::Absent | FullTextGenerationRetirement::Retired(_) => Ok(()),
    }
}

fn retire_failed_vector_generation(
    data_dir: &std::path::Path,
    generation: &str,
    retained: &BTreeSet<String>,
    expectation: SearchArtifactExpectation,
) -> Result<()> {
    if expectation == SearchArtifactExpectation::None {
        return Ok(());
    }
    let outcome = index_vector::try_retire_unpublished_generation(
        &data_dir.join("vector-index"),
        generation,
        retained,
    )
    .map_err(|_| vector_retirement_failure())?;
    match outcome {
        VectorGenerationRetirement::Deferred => Err(vector_retirement_failure()),
        VectorGenerationRetirement::Retired(summary)
            if expectation == SearchArtifactExpectation::Published
                && (!summary.removed_generation() || !summary.removed_generation_pin()) =>
        {
            Err(vector_retirement_failure())
        }
        VectorGenerationRetirement::Absent | VectorGenerationRetirement::Retired(_) => Ok(()),
    }
}

#[cfg(test)]
#[path = "search_publication_tests.rs"]
mod tests;
