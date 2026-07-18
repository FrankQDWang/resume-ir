pub fn crate_name() -> &'static str {
    "search-planner"
}

use std::fmt;

use core_domain::normalize_query_set_query;

#[derive(Clone, PartialEq, Eq)]
pub struct SearchPlan {
    query_text: String,
    terms: Vec<String>,
    limit: usize,
}

impl SearchPlan {
    pub const MAX_LIMIT: usize = 100;

    pub fn query_text(&self) -> &str {
        &self.query_text
    }

    pub fn terms(&self) -> &[String] {
        &self.terms
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

impl fmt::Debug for SearchPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchPlan")
            .field("query_text", &"<redacted>")
            .field("term_count", &self.terms.len())
            .field("limit", &self.limit)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchPlanError {
    EmptyQuery,
    InvalidQueryBounds,
}

impl fmt::Display for SearchPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchPlanError::EmptyQuery => formatter.write_str("search query is empty"),
            SearchPlanError::InvalidQueryBounds => {
                formatter.write_str("search query is outside semantic bounds")
            }
        }
    }
}

impl std::error::Error for SearchPlanError {}

pub fn plan_search(query: &str, limit: usize) -> Result<SearchPlan, SearchPlanError> {
    if query.trim().is_empty() {
        return Err(SearchPlanError::EmptyQuery);
    }

    let query_text = normalize_query_set_query(query).ok_or(SearchPlanError::InvalidQueryBounds)?;
    let terms = query_text
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();

    Ok(SearchPlan {
        query_text,
        terms,
        limit: limit.clamp(1, SearchPlan::MAX_LIMIT),
    })
}
