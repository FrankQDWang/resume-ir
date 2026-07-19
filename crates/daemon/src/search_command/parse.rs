use search_planner::plan_search;

use super::filters::parse_search_filters;
use crate::command_failure::CommandFailure;
use crate::search_contract::{DaemonSearchArgs, DaemonSearchMode};

pub(crate) fn parse_search_command(
    payload: &serde_json::Value,
) -> Result<DaemonSearchArgs, CommandFailure> {
    let object = payload.as_object().ok_or(CommandFailure::BadRequest(
        "search payload must be an object",
    ))?;
    const ALLOWED_FIELDS: &[&str] = &["query", "mode", "top_k", "filters"];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err(CommandFailure::BadRequest(
            "search payload contains an unknown field",
        ));
    }
    let query = payload
        .get("query")
        .and_then(serde_json::Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or(CommandFailure::BadRequest(
            "query must be a non-empty string",
        ))?
        .to_string();
    let mode = payload
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("fulltext");
    let mode =
        DaemonSearchMode::parse(mode).ok_or(CommandFailure::BadRequest("mode is invalid"))?;
    let top_k = match payload.get("top_k") {
        Some(value) => value
            .as_u64()
            .filter(|value| *value > 0)
            .and_then(|value| usize::try_from(value).ok())
            .map(|value| value.min(100))
            .ok_or(CommandFailure::BadRequest("top_k must be positive"))?,
        None => 10,
    };
    let query = plan_search(&query, top_k)
        .map_err(|_| CommandFailure::BadRequest("query is outside semantic bounds"))?
        .query_text()
        .to_string();
    let filter = parse_search_filters(payload.get("filters"))?;
    Ok(DaemonSearchArgs {
        query,
        mode,
        top_k,
        filter,
    })
}
