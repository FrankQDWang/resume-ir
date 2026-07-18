use crate::{
    ActiveSearchProjection, CandidateId, ContactHash, Document, EntityMention, EntityType,
};

mod hydration;
mod sql;
mod validation;

pub const MAX_SEARCH_FILTER_PREDICATES: usize = 32;
pub const MAX_SEARCH_FILTER_VALUES: usize = 64;
pub const MAX_BOUNDED_FILTER_SELECTION: usize = 100_000;
pub const MAX_EXACT_HIT_HYDRATION: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFilterCase {
    Exact,
    AsciiInsensitive,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchProjectionPredicate {
    EntityValuesAny {
        entity_type: EntityType,
        normalized_values: Vec<String>,
        min_confidence: f32,
        case: SearchFilterCase,
    },
    EntityValuesAnyOrMissing {
        entity_type: EntityType,
        normalized_values: Vec<String>,
        min_confidence: f32,
        case: SearchFilterCase,
    },
    NumericEntityMinimum {
        entity_type: EntityType,
        minimum: f32,
        min_confidence: f32,
    },
    DateRangeOverlap {
        start_month: i32,
        end_month: Option<i32>,
        min_confidence: f32,
    },
    ContactHashesAny(Vec<ContactHash>),
    MissingEntityType {
        entity_type: EntityType,
        min_confidence: f32,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchProjectionFilter {
    predicates: Vec<SearchProjectionPredicate>,
}

impl SearchProjectionFilter {
    pub fn new(
        predicates: Vec<SearchProjectionPredicate>,
    ) -> std::result::Result<Self, SearchProjectionFilterError> {
        let filter = Self { predicates };
        filter.validate()?;
        Ok(filter)
    }

    pub fn predicates(&self) -> &[SearchProjectionPredicate] {
        &self.predicates
    }

    fn validate(&self) -> std::result::Result<(), SearchProjectionFilterError> {
        if self.predicates.len() > MAX_SEARCH_FILTER_PREDICATES {
            return Err(SearchProjectionFilterError::TooManyPredicates);
        }
        for predicate in &self.predicates {
            validation::validate_predicate(predicate)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProjectionFilterError {
    TooManyPredicates,
    EmptyValues,
    TooManyValues,
    ValueTooLarge,
    InvalidConfidence,
    InvalidNumericMinimum,
    InvalidDateRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundedFilterSelection {
    Selected(Vec<ActiveSearchProjection>),
    TooLarge { cap: usize },
}

#[derive(Clone, PartialEq)]
pub struct SearchHitMetadata {
    pub projection: ActiveSearchProjection,
    pub document: Document,
    pub candidate_id: Option<CandidateId>,
    pub mentions: Vec<EntityMention>,
}

impl std::fmt::Debug for SearchHitMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchHitMetadata")
            .field("projection", &self.projection)
            .field("document", &"<redacted>")
            .field(
                "candidate_id",
                &self.candidate_id.as_ref().map(|_| "<redacted>"),
            )
            .field("mention_count", &self.mentions.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactHitHydration {
    Hydrated(Vec<SearchHitMetadata>),
    Failed(ExactHitHydrationFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExactHitHydrationFailure {
    pub position: Option<usize>,
    pub kind: ExactHitHydrationFailureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactHitHydrationFailureKind {
    Stale,
    NotFound,
    LimitExceeded(SearchHitMetadataLimit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchHitMetadataLimit {
    InputCount,
    DocumentMetadata,
    MentionsPerHit,
    TotalMentions,
    TotalStringBytes,
}
