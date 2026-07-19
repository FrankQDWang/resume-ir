use meta_store::{SearchProjectionServiceState, SearchRepairReason};

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
}

impl ServiceErrorCode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Repairing => "REPAIRING",
            Self::MetadataUnavailable => "METADATA_UNAVAILABLE",
            Self::QueryServiceUnavailable => "QUERY_SERVICE_UNAVAILABLE",
        }
    }

    pub(crate) fn action(self) -> &'static str {
        match self {
            Self::Repairing => "wait_for_repair",
            Self::MetadataUnavailable | Self::QueryServiceUnavailable => "retry",
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
    match services.aggregate() {
        ServiceState::Ready => serde_json::Value::Null,
        ServiceState::Repairing => serde_json::json!({
            "code": "REPAIRING",
            "action": "wait_for_repair",
        }),
        ServiceState::Degraded | ServiceState::Unavailable => serde_json::json!({
            "code": "QUERY_SERVICE_UNAVAILABLE",
            "action": "retry",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{ServiceHealth, ServiceState};

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
}
