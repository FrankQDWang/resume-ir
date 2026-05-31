use search_planner::{plan_search, SearchPlan};

#[test]
fn exposes_search_planner_crate_identity() {
    assert_eq!(search_planner::crate_name(), "search-planner");
}

#[test]
fn plans_mixed_query_without_echoing_raw_query_in_debug() {
    let plan = plan_search(" Java  支付  ", 25).unwrap();

    assert_eq!(plan.query_text(), "Java 支付");
    assert_eq!(plan.limit(), 25);
    assert_eq!(plan.terms(), &["Java", "支付"]);
    assert!(!format!("{plan:?}").contains("Java 支付"));
}

#[test]
fn rejects_empty_or_too_broad_queries_before_index_access() {
    assert!(plan_search("   ", 10).is_err());
    assert!(plan_search("的 and the", 10).is_err());
}

#[test]
fn clamps_limit_to_safe_top_n_bound() {
    let plan = plan_search("Rust", 10_000).unwrap();

    assert_eq!(plan.limit(), SearchPlan::MAX_LIMIT);
}
