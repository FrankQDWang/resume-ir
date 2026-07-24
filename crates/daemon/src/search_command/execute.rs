use std::time::Duration;

use crate::command_failure::CommandFailure;
use crate::query_runtime;
use crate::query_timing::{QueryStage, QueryStageTiming};
use crate::search_contract::{
    DaemonSearchArgs, DaemonSearchMode, SearchCancellation, SearchDeadline,
};
use crate::search_runtime_config::SearchRuntimeConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchCommandCompletion {
    Complete,
    Cancelled,
}

pub(crate) struct DaemonSearchOutput {
    pub(crate) request_id: String,
    pub(crate) completion: SearchCommandCompletion,
    pub(crate) visible_epoch: u64,
    pub(crate) mode: DaemonSearchMode,
    pub(crate) partial_reasons: Vec<&'static str>,
    pub(crate) elapsed: Duration,
    pub(crate) stage_timing: QueryStageTiming,
    pub(crate) hits: Vec<query_runtime::SearchHit>,
}

pub(crate) struct DaemonSearchExecution<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) args: &'a DaemonSearchArgs,
    pub(crate) query_parse_duration: Duration,
    pub(crate) deadline: &'a SearchDeadline,
    pub(crate) cancellation: &'a SearchCancellation,
}

pub(crate) fn execute_search_command(
    execution: &DaemonSearchExecution<'_>,
    config: &SearchRuntimeConfig,
    query_runtime: &mut query_runtime::DaemonQueryRuntime,
) -> Result<DaemonSearchOutput, CommandFailure> {
    let args = execution.args;
    let deadline = execution.deadline;
    let mut stage_timing = QueryStageTiming::default();
    stage_timing.record_duration(QueryStage::QueryParse, execution.query_parse_duration);
    if execution.cancellation.is_cancelled() {
        return Ok(daemon_search_cancelled_output(
            execution.request_id,
            0,
            args.mode,
            deadline.elapsed(),
            execution.query_parse_duration,
        ));
    }
    if deadline.expired() {
        return Ok(daemon_search_deadline_output(
            execution.request_id,
            0,
            args.mode,
            deadline.elapsed(),
            stage_timing,
            Vec::new(),
        ));
    }
    let outcome = query_runtime
        .execute(
            args,
            config,
            deadline,
            execution.cancellation,
            &mut stage_timing,
        )
        .map_err(|error| map_query_failure(error, args.mode))?;
    match outcome {
        query_runtime::SearchExecutionOutcome::Complete(search) => Ok(completed_search_output(
            execution.request_id,
            search.visible_epoch,
            args.mode,
            deadline.elapsed(),
            stage_timing,
            search.hits,
            search.partial_reasons,
        )),
        query_runtime::SearchExecutionOutcome::Cancelled { visible_epoch } => {
            Ok(daemon_search_cancelled_output(
                execution.request_id,
                visible_epoch,
                args.mode,
                deadline.elapsed(),
                execution.query_parse_duration,
            ))
        }
        query_runtime::SearchExecutionOutcome::DeadlineExceeded(search) => {
            Ok(daemon_search_deadline_output(
                execution.request_id,
                search.visible_epoch,
                args.mode,
                deadline.elapsed(),
                stage_timing,
                search.hits,
            ))
        }
    }
}

fn map_query_failure(error: query_runtime::QueryFailure, mode: DaemonSearchMode) -> CommandFailure {
    match error {
        query_runtime::QueryFailure::BadRequest => {
            CommandFailure::BadRequest("semantic query configuration is invalid")
        }
        query_runtime::QueryFailure::SelectionTooLarge => {
            CommandFailure::TooLarge("search filter selection exceeds the bounded limit")
        }
        query_runtime::QueryFailure::SemanticDisabled => {
            CommandFailure::ServiceUnavailable("SEMANTIC_DISABLED")
        }
        query_runtime::QueryFailure::Unavailable if mode == DaemonSearchMode::Semantic => {
            CommandFailure::ServiceUnavailable("SEMANTIC_RUNTIME_UNAVAILABLE")
        }
        query_runtime::QueryFailure::Integrity | query_runtime::QueryFailure::Unavailable => {
            CommandFailure::ServiceUnavailable("QUERY_SERVICE_UNAVAILABLE")
        }
    }
}

fn completed_search_output(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    stage_timing: QueryStageTiming,
    hits: Vec<query_runtime::SearchHit>,
    partial_reasons: Vec<&'static str>,
) -> DaemonSearchOutput {
    DaemonSearchOutput {
        request_id: request_id.to_string(),
        completion: SearchCommandCompletion::Complete,
        visible_epoch,
        mode,
        partial_reasons,
        elapsed,
        stage_timing,
        hits,
    }
}

pub(crate) fn daemon_search_deadline_output(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    stage_timing: QueryStageTiming,
    hits: Vec<query_runtime::SearchHit>,
) -> DaemonSearchOutput {
    completed_search_output(
        request_id,
        visible_epoch,
        mode,
        elapsed,
        stage_timing,
        hits,
        vec!["deadline_exceeded"],
    )
}

pub(crate) fn daemon_search_cancelled_output(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    query_parse_duration: Duration,
) -> DaemonSearchOutput {
    let mut stage_timing = QueryStageTiming::default();
    stage_timing.record_duration(QueryStage::QueryParse, query_parse_duration);
    DaemonSearchOutput {
        request_id: request_id.to_string(),
        completion: SearchCommandCompletion::Cancelled,
        visible_epoch,
        mode,
        partial_reasons: Vec::new(),
        elapsed,
        stage_timing,
        hits: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::map_query_failure;
    use crate::command_failure::CommandFailure;
    use crate::query_runtime::QueryFailure;
    use crate::search_contract::DaemonSearchMode;

    #[test]
    fn only_semantic_runtime_unavailability_gets_capability_classification() {
        assert!(matches!(
            map_query_failure(QueryFailure::Unavailable, DaemonSearchMode::Semantic),
            CommandFailure::ServiceUnavailable("SEMANTIC_RUNTIME_UNAVAILABLE")
        ));
        for (failure, mode) in [
            (QueryFailure::Integrity, DaemonSearchMode::Semantic),
            (QueryFailure::Unavailable, DaemonSearchMode::Hybrid),
            (QueryFailure::Unavailable, DaemonSearchMode::FullText),
        ] {
            assert!(matches!(
                map_query_failure(failure, mode),
                CommandFailure::ServiceUnavailable("QUERY_SERVICE_UNAVAILABLE")
            ));
        }
    }
}
