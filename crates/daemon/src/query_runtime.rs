use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use embedder::{EmbeddingBudget, EmbeddingInput, EmbeddingPriority};
use meta_store::{ActiveSearchProjection, CandidateId, ResumeVersionId, SearchSelection};
use rank_fusion::{fuse_hybrid_rrf, HybridRecall, RankedHit};
use search_runtime::{
    FullTextCandidate, HitLimit, HydratedSearchHit, QueryCoordinator, SearchRuntimeErrorCode,
    SelectionLimit, SemanticCandidate, SemanticContract, SemanticQueryVector,
};

use super::query_timing::{QueryStage, QueryStageTiming};
use crate::search_contract::{
    redact_search_file_name, DaemonSearchArgs, DaemonSearchMode, SearchCancellation, SearchDeadline,
};
use crate::search_runtime_config::SearchRuntimeConfig;

const FILTER_SELECTION_LIMIT: usize = meta_store::MAX_BOUNDED_FILTER_SELECTION;

pub(crate) struct DaemonQueryRuntime {
    coordinator: QueryCoordinator,
}

#[derive(Clone)]
pub(crate) struct SearchHit {
    pub(crate) rank: usize,
    pub(crate) selection: SearchSelection,
    pub(crate) file_name: String,
    pub(crate) snippet: String,
}

pub(crate) struct CompletedSearch {
    pub(crate) visible_epoch: u64,
    pub(crate) hits: Vec<SearchHit>,
    pub(crate) partial_reasons: Vec<&'static str>,
}

pub(crate) enum SearchExecutionOutcome {
    Complete(CompletedSearch),
    Cancelled { visible_epoch: u64 },
    DeadlineExceeded(CompletedSearch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryFailure {
    BadRequest,
    Integrity,
    SemanticDisabled,
    SelectionTooLarge,
    Unavailable,
}

enum PreparedSemantic {
    NotRequested,
    Ready {
        model_id: String,
        dimension: usize,
        query: SemanticQueryVector,
    },
    RuntimeUnavailable,
    ConfigurationMissing,
}

enum QueryPass {
    Complete(CompletedSearch),
    Cancelled { visible_epoch: u64 },
    DeadlineExceeded(CompletedSearch),
    SemanticDisabled,
    SemanticConfigurationMissing,
    SemanticContractMismatch,
    SemanticRuntimeUnavailable,
}

#[derive(Clone)]
struct RankedCandidate {
    projection: ActiveSearchProjection,
    score: f32,
    file_name: String,
    snippet: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FoldIdentity {
    Candidate(CandidateId),
    Version(ResumeVersionId),
}

impl DaemonQueryRuntime {
    pub(crate) fn open(data_dir: &Path) -> Result<Self, QueryFailure> {
        QueryCoordinator::open(data_dir)
            .map(|coordinator| Self { coordinator })
            .map_err(map_runtime_error)
    }

    pub(crate) fn execute(
        &mut self,
        args: &DaemonSearchArgs,
        config: &SearchRuntimeConfig,
        deadline: &SearchDeadline,
        cancellation: &SearchCancellation,
        stage_timing: &mut QueryStageTiming,
    ) -> Result<SearchExecutionOutcome, QueryFailure> {
        let semantic = prepare_semantic(args, config, deadline, cancellation)?;
        if cancellation.is_cancelled() {
            return Ok(SearchExecutionOutcome::Cancelled { visible_epoch: 0 });
        }
        if deadline.expired() {
            return Ok(SearchExecutionOutcome::DeadlineExceeded(CompletedSearch {
                visible_epoch: 0,
                hits: Vec::new(),
                partial_reasons: Vec::new(),
            }));
        }

        let candidate_limit = args.top_k.saturating_mul(5).clamp(args.top_k, 100);
        let hit_limit = HitLimit::new(candidate_limit).map_err(map_runtime_error)?;
        let selection_limit =
            SelectionLimit::new(FILTER_SELECTION_LIMIT).map_err(map_runtime_error)?;
        let pass = self
            .coordinator
            .with_query(|scope| {
                let visible_epoch = scope.visible_epoch();
                if cancellation.is_cancelled() {
                    return Ok(QueryPass::Cancelled { visible_epoch });
                }
                if deadline.expired() {
                    return Ok(QueryPass::DeadlineExceeded(CompletedSearch {
                        visible_epoch,
                        hits: Vec::new(),
                        partial_reasons: Vec::new(),
                    }));
                }

                let filter_selection = if args.filter.predicates().is_empty() {
                    None
                } else {
                    Some(stage_timing.measure(QueryStage::Prefilter, || {
                        scope.filter_selection(&args.filter, selection_limit)
                    })?)
                };

                let semantic_state =
                    validate_semantic_contract(scope.semantic_contract(), semantic);
                if let Some(terminal) = semantic_state.terminal {
                    if args.mode == DaemonSearchMode::Hybrid
                        && terminal == SemanticTerminal::RuntimeUnavailable
                    {
                        let lexical = fulltext_candidates(
                            &scope,
                            args,
                            hit_limit,
                            filter_selection.as_ref(),
                            stage_timing,
                        )?;
                        let hits = hydrate_candidates(&scope, lexical, args.top_k, stage_timing)?;
                        return Ok(QueryPass::Complete(CompletedSearch {
                            visible_epoch,
                            hits,
                            partial_reasons: vec!["embedding_runtime_unavailable"],
                        }));
                    }
                    return Ok(match terminal {
                        SemanticTerminal::Disabled => QueryPass::SemanticDisabled,
                        SemanticTerminal::ConfigurationMissing => {
                            QueryPass::SemanticConfigurationMissing
                        }
                        SemanticTerminal::ContractMismatch => QueryPass::SemanticContractMismatch,
                        SemanticTerminal::RuntimeUnavailable => {
                            QueryPass::SemanticRuntimeUnavailable
                        }
                    });
                }

                let candidates = match args.mode {
                    DaemonSearchMode::FullText => fulltext_candidates(
                        &scope,
                        args,
                        hit_limit,
                        filter_selection.as_ref(),
                        stage_timing,
                    )?,
                    DaemonSearchMode::Semantic => semantic_candidates(
                        &scope,
                        semantic_state.query.expect("validated semantic query"),
                        hit_limit,
                        filter_selection.as_ref(),
                        stage_timing,
                    )?,
                    DaemonSearchMode::Hybrid => {
                        let lexical = fulltext_candidates(
                            &scope,
                            args,
                            hit_limit,
                            filter_selection.as_ref(),
                            stage_timing,
                        )?;
                        if cancellation.is_cancelled() {
                            return Ok(QueryPass::Cancelled { visible_epoch });
                        }
                        let semantic = semantic_candidates(
                            &scope,
                            semantic_state.query.expect("validated semantic query"),
                            hit_limit,
                            filter_selection.as_ref(),
                            stage_timing,
                        )?;
                        stage_timing.measure(QueryStage::Fusion, || {
                            fuse_candidates(lexical, semantic, candidate_limit)
                        })
                    }
                };

                let hits = hydrate_candidates(&scope, candidates, args.top_k, stage_timing)?;
                let completed = CompletedSearch {
                    visible_epoch,
                    hits,
                    partial_reasons: Vec::new(),
                };
                if cancellation.is_cancelled() {
                    return Ok(QueryPass::Cancelled { visible_epoch });
                }
                if deadline.expired() {
                    return Ok(QueryPass::DeadlineExceeded(completed));
                }
                Ok(QueryPass::Complete(completed))
            })
            .map_err(map_runtime_error)?;

        match pass {
            QueryPass::Complete(search) => Ok(SearchExecutionOutcome::Complete(search)),
            QueryPass::Cancelled { visible_epoch } => {
                Ok(SearchExecutionOutcome::Cancelled { visible_epoch })
            }
            QueryPass::DeadlineExceeded(search) => {
                Ok(SearchExecutionOutcome::DeadlineExceeded(search))
            }
            QueryPass::SemanticDisabled => Err(QueryFailure::SemanticDisabled),
            QueryPass::SemanticConfigurationMissing => Err(QueryFailure::BadRequest),
            QueryPass::SemanticContractMismatch => Err(QueryFailure::Integrity),
            QueryPass::SemanticRuntimeUnavailable => Err(QueryFailure::Unavailable),
        }
    }
}

fn prepare_semantic(
    args: &DaemonSearchArgs,
    config: &SearchRuntimeConfig,
    deadline: &SearchDeadline,
    cancellation: &SearchCancellation,
) -> Result<PreparedSemantic, QueryFailure> {
    if args.mode == DaemonSearchMode::FullText {
        return Ok(PreparedSemantic::NotRequested);
    }
    let (Some(embedder), Some(model_id), Some(dimension)) = (
        config.resident_embedding.as_ref(),
        config.embedding_model_id.as_ref(),
        config.embedding_dimension,
    ) else {
        return Ok(PreparedSemantic::ConfigurationMissing);
    };
    let Some(remaining_ms) = deadline.remaining_ms() else {
        return Ok(PreparedSemantic::RuntimeUnavailable);
    };
    let embedding_timeout_ms = config.embedding_timeout_ms.min(remaining_ms);
    let deadline_limited = embedding_timeout_ms < config.embedding_timeout_ms;
    let input = EmbeddingInput::query("query", args.query.as_str());
    let vectors = match embedder.embed_batch_with_cancel(
        EmbeddingPriority::Interactive,
        &[input],
        EmbeddingBudget::new(1, args.query.len().max(1)),
        embedding_timeout_ms,
        || cancellation.is_cancelled(),
    ) {
        Ok(vectors) => vectors,
        Err(embedder::EmbeddingError::Cancelled) => {
            return Ok(PreparedSemantic::RuntimeUnavailable);
        }
        Err(embedder::EmbeddingError::Timeout) if deadline_limited => {
            return Ok(PreparedSemantic::RuntimeUnavailable);
        }
        Err(
            embedder::EmbeddingError::WorkerUnavailable
            | embedder::EmbeddingError::EngineFailed
            | embedder::EmbeddingError::Overloaded
            | embedder::EmbeddingError::Timeout,
        ) => return Ok(PreparedSemantic::RuntimeUnavailable),
        Err(_) => return Err(QueryFailure::Unavailable),
    };
    let vector = vectors
        .into_iter()
        .next()
        .ok_or(QueryFailure::Unavailable)?;
    let query = SemanticQueryVector::new(vector.values().to_vec()).map_err(map_runtime_error)?;
    Ok(PreparedSemantic::Ready {
        model_id: model_id.clone(),
        dimension,
        query,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SemanticTerminal {
    Disabled,
    ConfigurationMissing,
    ContractMismatch,
    RuntimeUnavailable,
}

struct ValidatedSemantic {
    query: Option<SemanticQueryVector>,
    terminal: Option<SemanticTerminal>,
}

fn validate_semantic_contract(
    contract: SemanticContract,
    prepared: PreparedSemantic,
) -> ValidatedSemantic {
    match (contract, prepared) {
        (_, PreparedSemantic::NotRequested) => ValidatedSemantic {
            query: None,
            terminal: None,
        },
        (SemanticContract::Disabled, _) => ValidatedSemantic {
            query: None,
            terminal: Some(SemanticTerminal::Disabled),
        },
        (SemanticContract::Enabled { .. }, PreparedSemantic::ConfigurationMissing) => {
            ValidatedSemantic {
                query: None,
                terminal: Some(SemanticTerminal::ConfigurationMissing),
            }
        }
        (SemanticContract::Enabled { .. }, PreparedSemantic::RuntimeUnavailable) => {
            ValidatedSemantic {
                query: None,
                terminal: Some(SemanticTerminal::RuntimeUnavailable),
            }
        }
        (
            SemanticContract::Enabled {
                model_id: expected_model,
                dimension: expected_dimension,
            },
            PreparedSemantic::Ready {
                model_id,
                dimension,
                query,
            },
        ) if model_id == expected_model && dimension == expected_dimension => ValidatedSemantic {
            query: Some(query),
            terminal: None,
        },
        (SemanticContract::Enabled { .. }, PreparedSemantic::Ready { .. }) => ValidatedSemantic {
            query: None,
            terminal: Some(SemanticTerminal::ContractMismatch),
        },
    }
}

fn fulltext_candidates(
    scope: &search_runtime::QueryScope<'_>,
    args: &DaemonSearchArgs,
    limit: HitLimit,
    selection: Option<&search_runtime::FilterSelection>,
    timing: &mut QueryStageTiming,
) -> Result<Vec<RankedCandidate>, search_runtime::SearchRuntimeError> {
    let hits = timing.measure(QueryStage::Bm25, || {
        scope.fulltext_candidates(&args.query, limit, selection)
    })?;
    timing.measure(QueryStage::Snippet, || {
        Ok(hits.into_iter().map(ranked_fulltext).collect())
    })
}

fn ranked_fulltext(hit: FullTextCandidate) -> RankedCandidate {
    RankedCandidate {
        projection: hit.projection,
        score: hit.score,
        file_name: hit.file_name,
        snippet: hit.snippet,
    }
}

fn semantic_candidates(
    scope: &search_runtime::QueryScope<'_>,
    query: SemanticQueryVector,
    limit: HitLimit,
    selection: Option<&search_runtime::FilterSelection>,
    timing: &mut QueryStageTiming,
) -> Result<Vec<RankedCandidate>, search_runtime::SearchRuntimeError> {
    let started = Instant::now();
    let hits = scope.semantic_candidates(query, limit, selection)?;
    timing.record_since(QueryStage::Ann, started);
    Ok(hits.into_iter().map(ranked_semantic).collect())
}

fn ranked_semantic(hit: SemanticCandidate) -> RankedCandidate {
    RankedCandidate {
        projection: hit.projection,
        score: hit.score,
        file_name: String::new(),
        snippet: "semantic match".to_string(),
    }
}

fn fuse_candidates(
    lexical: Vec<RankedCandidate>,
    semantic: Vec<RankedCandidate>,
    limit: usize,
) -> Vec<RankedCandidate> {
    let mut by_document = BTreeMap::<String, RankedCandidate>::new();
    for candidate in semantic.iter().chain(lexical.iter()) {
        by_document
            .entry(candidate.projection.document_id.to_string())
            .and_modify(|stored| {
                if stored.file_name.is_empty() && !candidate.file_name.is_empty() {
                    stored.file_name.clone_from(&candidate.file_name);
                    stored.snippet.clone_from(&candidate.snippet);
                }
            })
            .or_insert_with(|| candidate.clone());
    }
    let recall = HybridRecall::new(ranked_for_fusion(&lexical), ranked_for_fusion(&semantic));
    fuse_hybrid_rrf(recall, 60.0, limit)
        .into_iter()
        .filter_map(|ranked| {
            by_document.remove(ranked.doc_id()).map(|mut candidate| {
                candidate.score = ranked.score();
                candidate
            })
        })
        .collect()
}

fn ranked_for_fusion(candidates: &[RankedCandidate]) -> Vec<RankedHit> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            RankedHit::new(
                candidate.projection.document_id.to_string(),
                index + 1,
                candidate.score,
            )
        })
        .collect()
}

fn hydrate_candidates(
    scope: &search_runtime::QueryScope<'_>,
    candidates: Vec<RankedCandidate>,
    top_k: usize,
    timing: &mut QueryStageTiming,
) -> Result<Vec<SearchHit>, search_runtime::SearchRuntimeError> {
    let projections = candidates
        .iter()
        .map(|candidate| candidate.projection.clone())
        .collect::<Vec<_>>();
    let hydrated = timing.measure(QueryStage::BulkHydrate, || {
        scope.hydrate_exact_hits(&projections)
    })?;
    let mut seen = BTreeSet::new();
    let mut output = Vec::with_capacity(top_k.min(hydrated.len()));
    for (candidate, metadata) in candidates.into_iter().zip(hydrated) {
        if metadata.selection.document_id != candidate.projection.document_id
            || metadata.selection.resume_version_id != candidate.projection.resume_version_id
        {
            return Err(search_runtime::SearchRuntimeError::integrity_violation());
        }
        let fold = fold_identity(&metadata);
        if !seen.insert(fold) {
            continue;
        }
        output.push(SearchHit {
            rank: output.len() + 1,
            selection: metadata.selection,
            file_name: redact_search_file_name(if candidate.file_name.is_empty() {
                &metadata.document.file_name
            } else {
                &candidate.file_name
            }),
            snippet: candidate.snippet,
        });
        if output.len() == top_k {
            break;
        }
    }
    Ok(output)
}

fn fold_identity(hit: &HydratedSearchHit) -> FoldIdentity {
    hit.candidate_id
        .clone()
        .map(FoldIdentity::Candidate)
        .unwrap_or_else(|| FoldIdentity::Version(hit.selection.resume_version_id.clone()))
}

fn map_runtime_error(error: search_runtime::SearchRuntimeError) -> QueryFailure {
    match error.code() {
        SearchRuntimeErrorCode::Unavailable => QueryFailure::Unavailable,
        SearchRuntimeErrorCode::Integrity => QueryFailure::Integrity,
        SearchRuntimeErrorCode::SemanticDisabled => QueryFailure::SemanticDisabled,
        SearchRuntimeErrorCode::SelectionTooLarge => QueryFailure::SelectionTooLarge,
        SearchRuntimeErrorCode::InvalidRequest => QueryFailure::BadRequest,
    }
}
