pub fn crate_name() -> &'static str {
    "search-planner"
}

use std::fmt;

const STOP_WORDS: &[&str] = &["and", "the", "or", "的", "了", "和"];

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
    NoSearchableTerms,
}

impl fmt::Display for SearchPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchPlanError::EmptyQuery => formatter.write_str("search query is empty"),
            SearchPlanError::NoSearchableTerms => {
                formatter.write_str("search query has no searchable terms")
            }
        }
    }
}

impl std::error::Error for SearchPlanError {}

pub fn plan_search(query: &str, limit: usize) -> Result<SearchPlan, SearchPlanError> {
    let query_text = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query_text.is_empty() {
        return Err(SearchPlanError::EmptyQuery);
    }

    let terms = query_text
        .split_whitespace()
        .filter(|term| !is_stop_word(term))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(SearchPlanError::NoSearchableTerms);
    }

    Ok(SearchPlan {
        query_text,
        terms,
        limit: limit.clamp(1, SearchPlan::MAX_LIMIT),
    })
}

fn is_stop_word(term: &str) -> bool {
    let normalized = term.to_ascii_lowercase();
    STOP_WORDS.iter().any(|stop_word| *stop_word == normalized)
}
