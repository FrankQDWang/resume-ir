use meta_store::{
    ArtifactRepairAttemptErrorKind, ArtifactRepairAttemptPhase, ArtifactRepairAttemptState,
    SearchProjectionServiceState, SearchProjectionState, SearchRepairReason,
    ARTIFACT_REPAIR_MAX_ATTEMPTS,
};

const MAX_REPAIR_RETRY_AFTER_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceState {
    Ready,
    Degraded,
    Repairing,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceErrorCode {
    Repairing,
    MetadataUnavailable,
    QueryServiceUnavailable,
    QueryServiceRepairRequired,
}

impl ServiceErrorCode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Repairing => "REPAIRING",
            Self::MetadataUnavailable => "METADATA_UNAVAILABLE",
            Self::QueryServiceUnavailable | Self::QueryServiceRepairRequired => {
                "QUERY_SERVICE_UNAVAILABLE"
            }
        }
    }

    pub(crate) fn action(self) -> &'static str {
        match self {
            Self::Repairing => "wait_for_repair",
            Self::MetadataUnavailable | Self::QueryServiceUnavailable => "retry",
            Self::QueryServiceRepairRequired => "repair_required",
        }
    }
}

impl ServiceState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Repairing => "repairing",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServiceHealth {
    pub(crate) metadata: ServiceState,
    pub(crate) query: ServiceState,
}

impl ServiceHealth {
    pub(crate) fn aggregate(self) -> ServiceState {
        match (self.metadata, self.query) {
            (ServiceState::Unavailable, _) | (_, ServiceState::Unavailable) => {
                ServiceState::Degraded
            }
            (ServiceState::Repairing, _) | (_, ServiceState::Repairing) => ServiceState::Repairing,
            (ServiceState::Degraded, _) | (_, ServiceState::Degraded) => ServiceState::Degraded,
            (ServiceState::Ready, ServiceState::Ready) => ServiceState::Ready,
        }
    }
}

pub(crate) fn projection_service_health(state: SearchProjectionServiceState) -> ServiceHealth {
    ServiceHealth {
        metadata: ServiceState::Ready,
        query: match state {
            SearchProjectionServiceState::Ready => ServiceState::Ready,
            SearchProjectionServiceState::Repairing => ServiceState::Repairing,
            SearchProjectionServiceState::RepairBlocked => ServiceState::Unavailable,
        },
    }
}

pub(crate) fn search_repair_reason_label(reason: SearchRepairReason) -> &'static str {
    match reason {
        SearchRepairReason::MigrationRebuild => "migration_rebuild",
        SearchRepairReason::ArtifactUnavailable => "artifact_unavailable",
        SearchRepairReason::SourceUnavailable => "source_unavailable",
        SearchRepairReason::RuntimeInvariant => "runtime_invariant",
    }
}

pub(crate) fn service_error_json(services: ServiceHealth) -> serde_json::Value {
    match (services.metadata, services.query) {
        (ServiceState::Ready, ServiceState::Ready) => serde_json::Value::Null,
        (ServiceState::Ready, ServiceState::Repairing) => serde_json::json!({
            "code": "REPAIRING",
            "action": "wait_for_repair",
        }),
        (ServiceState::Ready, ServiceState::Unavailable) => serde_json::json!({
            "code": "QUERY_SERVICE_UNAVAILABLE",
            "action": "repair_required",
        }),
        _ => serde_json::json!({
            "code": "METADATA_UNAVAILABLE",
            "action": "retry",
        }),
    }
}

pub(crate) fn repair_progress_json(
    projection: &SearchProjectionState,
    attempt: Option<&ArtifactRepairAttemptState>,
    now_seconds: i64,
) -> serde_json::Value {
    if projection.service_state == SearchProjectionServiceState::Ready {
        return serde_json::Value::Null;
    }
    let phase = match (
        projection.service_state,
        projection.repair_reason,
        attempt.map(|attempt| attempt.phase),
    ) {
        (_, Some(SearchRepairReason::SourceUnavailable), _) => {
            RepairProgressPhase::SourceUnavailable
        }
        (SearchProjectionServiceState::RepairBlocked, _, _) => RepairProgressPhase::Blocked,
        (_, Some(SearchRepairReason::MigrationRebuild), _) => RepairProgressPhase::MigrationRebuild,
        (
            _,
            Some(SearchRepairReason::ArtifactUnavailable),
            Some(ArtifactRepairAttemptPhase::Running),
        ) => RepairProgressPhase::Rebuilding,
        (
            _,
            Some(SearchRepairReason::ArtifactUnavailable),
            Some(ArtifactRepairAttemptPhase::RetryWait),
        ) => RepairProgressPhase::RetryWait,
        _ => RepairProgressPhase::Queued,
    };
    let visible_attempt = phase.exposes_attempt().then_some(attempt).flatten();
    let attempt_count = visible_attempt.map(|attempt| attempt.attempt_count);
    let retry_after_ms = if phase == RepairProgressPhase::RetryWait {
        attempt
            .and_then(|attempt| attempt.next_retry_at)
            .map(|deadline| {
                deadline
                    .as_unix_seconds()
                    .saturating_sub(now_seconds)
                    .max(0)
                    .saturating_mul(1_000)
                    .min(MAX_REPAIR_RETRY_AFTER_MS)
            })
    } else {
        None
    };
    let max_attempts = phase
        .is_attempt_bounded()
        .then_some(ARTIFACT_REPAIR_MAX_ATTEMPTS);
    let last_error_kind = phase
        .exposes_last_error()
        .then_some(attempt)
        .flatten()
        .and_then(|attempt| attempt.last_error_kind);
    serde_json::json!({
        "phase": phase.label(),
        "attempt": attempt_count,
        "max_attempts": max_attempts,
        "retry_after_ms": retry_after_ms,
        "last_error_kind": last_error_kind.map(ArtifactRepairAttemptErrorKind::label),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepairProgressPhase {
    Queued,
    MigrationRebuild,
    SourceUnavailable,
    Rebuilding,
    RetryWait,
    Blocked,
}

impl RepairProgressPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::MigrationRebuild => "migration_rebuild",
            Self::SourceUnavailable => "source_unavailable",
            Self::Rebuilding => "rebuilding",
            Self::RetryWait => "retry_wait",
            Self::Blocked => "blocked",
        }
    }

    fn exposes_attempt(self) -> bool {
        matches!(self, Self::Rebuilding | Self::RetryWait | Self::Blocked)
    }

    fn exposes_last_error(self) -> bool {
        matches!(self, Self::RetryWait | Self::Blocked)
    }

    fn is_attempt_bounded(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Rebuilding | Self::RetryWait | Self::Blocked
        )
    }
}

#[cfg(test)]
mod tests {
    use meta_store::{
        ArtifactRepairAttemptErrorKind, ArtifactRepairAttemptPhase, ArtifactRepairAttemptState,
        SearchProjectionServiceState, SearchProjectionState, SearchRepairReason, UnixTimestamp,
    };

    use super::{
        projection_service_health, repair_progress_json, service_error_json, ServiceHealth,
        ServiceState,
    };

    #[test]
    fn aggregate_never_hides_an_unavailable_dependency() {
        assert_eq!(
            ServiceHealth {
                metadata: ServiceState::Ready,
                query: ServiceState::Unavailable,
            }
            .aggregate(),
            ServiceState::Degraded
        );
    }

    #[test]
    fn blocked_repair_requires_remediation_and_retry_wait_is_bounded() {
        let projection = SearchProjectionState {
            service_state: SearchProjectionServiceState::Repairing,
            generation: Some("synthetic-generation".to_string()),
            visible_epoch: 7,
            repair_reason: Some(SearchRepairReason::ArtifactUnavailable),
            publication: None,
            updated_at: UnixTimestamp::from_unix_seconds(100),
        };
        let attempt = ArtifactRepairAttemptState {
            attempt_count: 2,
            phase: ArtifactRepairAttemptPhase::RetryWait,
            started_at: UnixTimestamp::from_unix_seconds(100),
            next_retry_at: Some(UnixTimestamp::from_unix_seconds(104)),
            last_error_kind: Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy),
        };
        let progress = repair_progress_json(&projection, Some(&attempt), 101);
        assert_eq!(progress["phase"], "retry_wait");
        assert_eq!(progress["attempt"], 2);
        assert_eq!(progress["max_attempts"], 5);
        assert_eq!(progress["retry_after_ms"], 3_000);
        assert_eq!(progress["last_error_kind"], "fulltext_publication_busy");

        let rollback_progress = repair_progress_json(
            &projection,
            Some(&ArtifactRepairAttemptState {
                next_retry_at: Some(UnixTimestamp::from_unix_seconds(10_000)),
                ..attempt.clone()
            }),
            101,
        );
        assert_eq!(rollback_progress["retry_after_ms"], 60_000);

        let blocked_projection = SearchProjectionState {
            service_state: SearchProjectionServiceState::RepairBlocked,
            generation: Some("synthetic-generation".to_string()),
            visible_epoch: 7,
            repair_reason: Some(SearchRepairReason::RuntimeInvariant),
            publication: None,
            updated_at: UnixTimestamp::from_unix_seconds(104),
        };
        let exhausted_attempt = ArtifactRepairAttemptState {
            attempt_count: 5,
            phase: ArtifactRepairAttemptPhase::RetryWait,
            started_at: UnixTimestamp::from_unix_seconds(100),
            next_retry_at: Some(UnixTimestamp::from_unix_seconds(164)),
            last_error_kind: Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy),
        };
        assert_eq!(
            repair_progress_json(&blocked_projection, Some(&exhausted_attempt), 104),
            serde_json::json!({
                "phase": "blocked",
                "attempt": 5,
                "max_attempts": 5,
                "retry_after_ms": null,
                "last_error_kind": "fulltext_publication_busy",
            })
        );

        let source_blocked_projection = SearchProjectionState {
            repair_reason: Some(SearchRepairReason::SourceUnavailable),
            ..blocked_projection
        };
        assert_eq!(
            repair_progress_json(&source_blocked_projection, Some(&exhausted_attempt), 104),
            serde_json::json!({
                "phase": "source_unavailable",
                "attempt": null,
                "max_attempts": null,
                "retry_after_ms": null,
                "last_error_kind": null,
            })
        );

        let blocked = projection_service_health(SearchProjectionServiceState::RepairBlocked);
        assert_eq!(
            service_error_json(blocked),
            serde_json::json!({
                "code": "QUERY_SERVICE_UNAVAILABLE",
                "action": "repair_required",
            })
        );
    }
}
