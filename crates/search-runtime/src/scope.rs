use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::rc::Rc;

use core_domain::{ActiveSearchProjection, DocumentId, ResumeVersionId};
use index_fulltext::{FullTextIndex, SearchQuery};
use index_vector::{QueryVector, VectorModelContract, VectorSnapshotReader};
use meta_store::{
    BoundedFilterSelection, CandidateId, Document, EntityMention, ExactHitHydration,
    SearchMetadataSnapshot, SearchProjectionFilter, SearchSelection, MAX_BOUNDED_FILTER_SELECTION,
    MAX_EXACT_HIT_HYDRATION,
};

use crate::SearchRuntimeError;

const MAX_QUERY_HITS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitLimit(NonZeroUsize);

impl HitLimit {
    pub fn new(value: usize) -> Result<Self, SearchRuntimeError> {
        let value = NonZeroUsize::new(value).ok_or_else(SearchRuntimeError::invalid_request)?;
        if value.get() > MAX_QUERY_HITS {
            return Err(SearchRuntimeError::invalid_request());
        }
        Ok(Self(value))
    }

    pub fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionLimit(NonZeroUsize);

impl SelectionLimit {
    pub fn new(value: usize) -> Result<Self, SearchRuntimeError> {
        let value = NonZeroUsize::new(value).ok_or_else(SearchRuntimeError::invalid_request)?;
        if value.get() > MAX_BOUNDED_FILTER_SELECTION {
            return Err(SearchRuntimeError::invalid_request());
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticContract {
    Disabled,
    Enabled { model_id: String, dimension: usize },
}

/// Validated query embedding accepted by the composite search facade.
///
/// Keeping the physical vector type private prevents query consumers from
/// bypassing the generation-pinned coordinator and opening vector artifacts
/// directly.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticQueryVector(QueryVector);

impl SemanticQueryVector {
    pub fn new(values: Vec<f32>) -> Result<Self, SearchRuntimeError> {
        QueryVector::new(values)
            .map(Self)
            .map_err(|_| SearchRuntimeError::invalid_request())
    }
}

pub struct FilterSelection {
    projections: Vec<ActiveSearchProjection>,
    document_ids: BTreeSet<String>,
}

impl std::fmt::Debug for FilterSelection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilterSelection")
            .field("projection_count", &self.projections.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct FullTextCandidate {
    pub projection: ActiveSearchProjection,
    pub rank: usize,
    pub score: f32,
    pub file_name: String,
    pub snippet: String,
}

impl std::fmt::Debug for FullTextCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FullTextCandidate")
            .field("projection", &self.projection)
            .field("rank", &self.rank)
            .field("score", &self.score)
            .field("file_name", &"<redacted>")
            .field("snippet", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticCandidate {
    pub projection: ActiveSearchProjection,
    pub rank: usize,
    pub score: f32,
}

#[derive(Clone, PartialEq)]
pub struct HydratedSearchHit {
    pub selection: SearchSelection,
    pub document: Document,
    pub candidate_id: Option<CandidateId>,
    pub mentions: Vec<EntityMention>,
}

impl std::fmt::Debug for HydratedSearchHit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HydratedSearchHit")
            .field("selection", &self.selection)
            .field("document", &"<redacted>")
            .field(
                "candidate_id",
                &self.candidate_id.as_ref().map(|_| "<redacted>"),
            )
            .field("mention_count", &self.mentions.len())
            .finish()
    }
}

pub struct QueryScope<'query> {
    metadata: &'query SearchMetadataSnapshot<'query>,
    fulltext: &'query FullTextIndex,
    vector: &'query VectorSnapshotReader,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'query> QueryScope<'query> {
    pub(crate) fn new(
        metadata: &'query SearchMetadataSnapshot<'query>,
        fulltext: &'query FullTextIndex,
        vector: &'query VectorSnapshotReader,
    ) -> Self {
        Self {
            metadata,
            fulltext,
            vector,
            _not_send_or_sync: PhantomData,
        }
    }

    pub fn visible_epoch(&self) -> u64 {
        self.metadata.head().visible_epoch
    }

    pub fn semantic_contract(&self) -> SemanticContract {
        match self.vector.summary().model_contract() {
            VectorModelContract::Disabled => SemanticContract::Disabled,
            VectorModelContract::Enabled {
                model_id,
                dimension,
            } => SemanticContract::Enabled {
                model_id: model_id.clone(),
                dimension: *dimension,
            },
        }
    }

    pub fn filter_selection(
        &self,
        filter: &SearchProjectionFilter,
        limit: SelectionLimit,
    ) -> Result<FilterSelection, SearchRuntimeError> {
        let selection = self
            .metadata
            .bounded_filter_selection(filter, limit.0)
            .map_err(|_| SearchRuntimeError::integrity())?;
        match selection {
            BoundedFilterSelection::TooLarge { .. } => {
                Err(SearchRuntimeError::selection_too_large())
            }
            BoundedFilterSelection::Selected(projections) => {
                let document_ids = projections
                    .iter()
                    .map(|projection| projection.document_id.to_string())
                    .collect();
                Ok(FilterSelection {
                    projections,
                    document_ids,
                })
            }
        }
    }

    pub fn fulltext_candidates(
        &self,
        query: &str,
        limit: HitLimit,
        selection: Option<&FilterSelection>,
    ) -> Result<Vec<FullTextCandidate>, SearchRuntimeError> {
        if query.trim().is_empty() {
            return Err(SearchRuntimeError::invalid_request());
        }
        let query = SearchQuery::new(query).with_limit(limit.get());
        let hits = match selection {
            Some(selection) => self
                .fulltext
                .search_allowed_doc_ids(query, &selection.document_ids),
            None => self.fulltext.search(query),
        }
        .map_err(|_| SearchRuntimeError::unavailable())?;
        hits.into_iter()
            .map(|hit| {
                let projection = parse_projection(&hit.doc_id, &hit.resume_version_id)?;
                if selection.is_some_and(|selection| !selection.contains(&projection)) {
                    return Err(SearchRuntimeError::integrity());
                }
                Ok(FullTextCandidate {
                    projection,
                    rank: hit.rank,
                    score: hit.score,
                    file_name: hit.file_name,
                    snippet: hit.snippet,
                })
            })
            .collect()
    }

    pub fn semantic_candidates(
        &self,
        query: SemanticQueryVector,
        limit: HitLimit,
        selection: Option<&FilterSelection>,
    ) -> Result<Vec<SemanticCandidate>, SearchRuntimeError> {
        if matches!(
            self.vector.summary().model_contract(),
            VectorModelContract::Disabled
        ) {
            return Err(SearchRuntimeError::semantic_disabled());
        }
        let hits = self
            .vector
            .knn(query.0, limit.get())
            .map_err(|_| SearchRuntimeError::invalid_request())?;
        hits.into_iter()
            .enumerate()
            .filter_map(|(rank, hit)| {
                let projection = parse_projection(hit.document_id(), hit.resume_version_id());
                match projection {
                    Ok(projection)
                        if selection.is_none_or(|selection| selection.contains(&projection)) =>
                    {
                        Some(Ok(SemanticCandidate {
                            projection,
                            rank: rank + 1,
                            score: hit.score(),
                        }))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect()
    }

    pub fn hydrate_exact_hits(
        &self,
        projections: &[ActiveSearchProjection],
    ) -> Result<Vec<HydratedSearchHit>, SearchRuntimeError> {
        let cap =
            NonZeroUsize::new(MAX_EXACT_HIT_HYDRATION).ok_or_else(SearchRuntimeError::integrity)?;
        match self
            .metadata
            .hydrate_exact_hits(projections, cap)
            .map_err(|_| SearchRuntimeError::integrity())?
        {
            ExactHitHydration::Failed(_) => Err(SearchRuntimeError::integrity()),
            ExactHitHydration::Hydrated(hits) => Ok(hits
                .into_iter()
                .map(|hit| HydratedSearchHit {
                    selection: SearchSelection {
                        document_id: hit.projection.document_id,
                        resume_version_id: hit.projection.resume_version_id,
                        visible_epoch: self.visible_epoch(),
                    },
                    document: hit.document,
                    candidate_id: hit.candidate_id,
                    mentions: hit.mentions,
                })
                .collect()),
        }
    }
}

impl FilterSelection {
    fn contains(&self, projection: &ActiveSearchProjection) -> bool {
        self.projections
            .binary_search_by(|candidate| {
                candidate
                    .document_id
                    .cmp(&projection.document_id)
                    .then_with(|| {
                        candidate
                            .resume_version_id
                            .cmp(&projection.resume_version_id)
                    })
            })
            .is_ok()
    }
}

fn parse_projection(
    document_id: &str,
    resume_version_id: &str,
) -> Result<ActiveSearchProjection, SearchRuntimeError> {
    Ok(ActiveSearchProjection {
        document_id: document_id
            .parse::<DocumentId>()
            .map_err(|_| SearchRuntimeError::integrity())?,
        resume_version_id: resume_version_id
            .parse::<ResumeVersionId>()
            .map_err(|_| SearchRuntimeError::integrity())?,
    })
}
