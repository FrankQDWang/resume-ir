#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    pub query: String,
    pub top_k: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPlan {
    pub fulltext_query: String,
    pub top_k: usize,
    pub include_snippet: bool,
}

#[must_use]
pub fn plan_search(request: SearchRequest) -> SearchPlan {
    let top_k = match request.top_k {
        0 => 20,
        value => value.min(100),
    };
    SearchPlan {
        fulltext_query: request.query,
        top_k,
        include_snippet: true,
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "search-planner"
}
