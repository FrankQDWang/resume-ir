use serde::{de::IgnoredAny, Deserialize, Serialize};

use super::enums::{
    EvidenceLane, EvidenceStatus, PrivacyBoundary, ScanErrorClass, ScanErrorOperation,
    SourceRootDeletionErrorCode, SourceRootDeletionPhase,
};
use super::health_contract::{
    deserialize_required_nullable, status_for_core, validate_counts, validate_health_contract,
    validate_latency, validate_repair_progress, Capabilities, CoreError, CoreStatus, IpcMetrics,
    OptionalRuntimes, ProcessState, RepairProgress, WriterHealth,
};
use super::{decode, ensure, ensure_schema, DesktopError, SafeCount};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticsBody {
    schema_version: String,
    privacy_boundary: PrivacyBoundary,
    evidence_lane: EvidenceLane,
    evidence_status: EvidenceStatus,
    contains_raw_resume_text: bool,
    contains_queries: bool,
    contains_resume_paths: bool,
    contains_candidate_results: bool,
    contains_snippet_text: bool,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    visible_epoch: Option<SafeCount>,
    process_state: ProcessState,
    core: CoreStatus,
    optional_runtimes: OptionalRuntimes,
    writer: WriterHealth,
    capabilities: Capabilities,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    repair_progress: Option<RepairProgress>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    error: Option<CoreError>,
    metrics: DiagnosticsMetrics,
    error_counts: DiagnosticsErrorCounts,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    source_root_deletion_attempts: Option<Vec<SourceRootDeletionAttempt>>,
    #[serde(skip_serializing)]
    benchmark_refs: Vec<IgnoredAny>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsMetrics {
    ipc: IpcMetrics,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    indexed_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    searchable_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    partial_documents: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    embedding_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    recovery_queue_depth: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_queued: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_recoverable: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_tasks_cancelled: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    source_roots_total: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    source_roots_active: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    source_roots_offline: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    source_root_deletions_in_progress: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    query_latency: Option<QueryLatency>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QueryLatency {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    sample_count: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p50_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p95_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    p99_ms: Option<f64>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    last_result_count: Option<SafeCount>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsErrorCounts {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    failed_retryable: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    failed_permanent: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    import_scan_errors: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_page_budget_blocked: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    ocr_language_unavailable: Option<SafeCount>,
    scan_error_buckets: Vec<ScanErrorBucket>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScanErrorBucket {
    class: ScanErrorClass,
    operation: ScanErrorOperation,
    count: SafeCount,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceRootDeletionAttempt {
    phase: SourceRootDeletionPhase,
    attempt_count: SafeCount,
    last_attempt_at: SafeCount,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    last_error_phase: Option<SourceRootDeletionPhase>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    last_error_code: Option<SourceRootDeletionErrorCode>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    last_error_at: Option<SafeCount>,
}

pub(super) fn project_diagnostics(body: &[u8]) -> Result<DiagnosticsBody, DesktopError> {
    let value: DiagnosticsBody = decode(body)?;
    ensure_schema(&value.schema_version, "resume-ir.diagnostics.v11")?;
    validate_health_contract(
        status_for_core(value.core.state),
        &value.core,
        &value.optional_runtimes,
        &value.writer,
        &value.capabilities,
        value.error.as_ref(),
    )?;
    validate_counts(
        value.visible_epoch.is_some(),
        [
            value.metrics.indexed_documents.is_some(),
            value.metrics.searchable_documents.is_some(),
            value.metrics.partial_documents.is_some(),
            value.metrics.ocr_queue_depth.is_some(),
            value.metrics.embedding_queue_depth.is_some(),
            value.metrics.recovery_queue_depth.is_some(),
            value.metrics.import_tasks_queued.is_some(),
            value.metrics.import_tasks_recoverable.is_some(),
            value.metrics.import_tasks_cancelled.is_some(),
            value.metrics.source_roots_total.is_some(),
            value.metrics.source_roots_active.is_some(),
            value.metrics.source_roots_offline.is_some(),
            value.metrics.source_root_deletions_in_progress.is_some(),
            value
                .metrics
                .query_latency
                .as_ref()
                .and_then(|latency| latency.sample_count)
                .is_some(),
            value.error_counts.failed_retryable.is_some(),
            value.error_counts.failed_permanent.is_some(),
            value.error_counts.import_scan_errors.is_some(),
            value.error_counts.ocr_page_budget_blocked.is_some(),
            value.error_counts.ocr_language_unavailable.is_some(),
        ],
    )?;
    if let (Some(total), Some(active), Some(offline), Some(deleting)) = (
        value.metrics.source_roots_total,
        value.metrics.source_roots_active,
        value.metrics.source_roots_offline,
        value.metrics.source_root_deletions_in_progress,
    ) {
        ensure(
            total.value() <= 16
                && active.value().saturating_add(offline.value()) <= total.value()
                && deleting.value() <= total.value(),
        )?;
    }
    validate_repair_progress(value.core.state, value.repair_progress.as_ref())?;
    ensure(
        value.source_root_deletion_attempts.is_some() == value.visible_epoch.is_some(),
    )?;
    if let Some(attempts) = &value.source_root_deletion_attempts {
        ensure(attempts.len() <= 16)?;
        for attempt in attempts {
            let error_fields = [
                attempt.last_error_phase.is_some(),
                attempt.last_error_code.is_some(),
                attempt.last_error_at.is_some(),
            ];
            ensure(
                attempt.attempt_count.value() > 0
                    && (error_fields.iter().all(|present| *present)
                        || error_fields.iter().all(|present| !*present))
                    && attempt
                        .last_error_at
                        .is_none_or(|failed_at| failed_at.value() >= attempt.last_attempt_at.value()),
            )?;
        }
    }
    ensure(
        !value.contains_raw_resume_text
            && !value.contains_queries
            && !value.contains_resume_paths
            && !value.contains_candidate_results
            && !value.contains_snippet_text
            && value.benchmark_refs.len() <= 64
            && value.error_counts.scan_error_buckets.len() <= 16,
    )?;
    if let Some(latency) = &value.metrics.query_latency {
        validate_latency(latency.p50_ms, latency.p95_ms, latency.p99_ms)?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_v11_requires_the_full_daemon_shape_and_redacts_benchmark_refs() {
        let raw = include_str!("../../tests/fixtures/daemon-diagnostics-v11-ready.json");
        let projected = project_diagnostics(raw.as_bytes()).unwrap();
        let exposed = serde_json::to_value(projected).unwrap();
        assert_eq!(exposed["schema_version"], "resume-ir.diagnostics.v11");
        assert!(exposed.get("benchmark_refs").is_none());

        let mut with_attempt: serde_json::Value = serde_json::from_str(raw).unwrap();
        with_attempt["source_root_deletion_attempts"] = serde_json::json!([{
            "phase": "quiescing",
            "attempt_count": 2,
            "last_attempt_at": 1_800_300_301_u64,
            "last_error_phase": "quiescing",
            "last_error_code": "ocr_quiescence_timeout",
            "last_error_at": 1_800_300_302_u64,
        }]);
        assert!(project_diagnostics(&serde_json::to_vec(&with_attempt).unwrap()).is_ok());
        with_attempt["source_root_deletion_attempts"][0]["phase"] =
            serde_json::json!("complete");
        assert!(project_diagnostics(&serde_json::to_vec(&with_attempt).unwrap()).is_err());
        with_attempt["source_root_deletion_attempts"][0]["phase"] =
            serde_json::json!("quiescing");
        for retired_code in ["receipt_unavailable", "service_unavailable"] {
            with_attempt["source_root_deletion_attempts"][0]["last_error_code"] =
                serde_json::json!(retired_code);
            assert!(project_diagnostics(&serde_json::to_vec(&with_attempt).unwrap()).is_err());
        }

        let mut missing: serde_json::Value = serde_json::from_str(raw).unwrap();
        missing.as_object_mut().unwrap().remove("benchmark_refs");
        assert!(project_diagnostics(&serde_json::to_vec(&missing).unwrap()).is_err());
        let mut extra: serde_json::Value = serde_json::from_str(raw).unwrap();
        extra["private_debug"] = serde_json::json!(true);
        assert!(project_diagnostics(&serde_json::to_vec(&extra).unwrap()).is_err());
        let mut legacy: serde_json::Value = serde_json::from_str(raw).unwrap();
        legacy["schema_version"] = serde_json::json!("resume-ir.diagnostics.v10");
        assert!(project_diagnostics(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }
}
