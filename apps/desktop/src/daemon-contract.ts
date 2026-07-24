import type {
  CapabilityReason,
  CapabilityStatus,
  CoreReason,
  DaemonReply,
  DiagnosticsBody,
  OptionalRuntimeStatus,
  StatusBody,
} from "./daemon"

const coreStates = ["initializing", "ready", "repairing", "degraded", "blocked"] as const
const coreReasons = ["metadata_initializing", "migration_rebuild", "artifact_unavailable", "source_unavailable", "runtime_invariant", "unsupported_store_schema", "metadata_unavailable"] as const
const runtimeStates = ["initializing", "available", "unavailable"] as const
const runtimeReasons = ["missing", "invalid", "start_failed", "not_configured"] as const
const capabilityStates = ["initializing", "available", "degraded", "unavailable", "blocked"] as const
const capabilityReasons = ["core_initializing", "core_blocked", "embedding_unavailable", "ocr_unavailable", "classifier_unavailable"] as const
const capabilityNames = ["keyword_search", "detail", "semantic_search", "hybrid_search", "text_import", "ocr_import", "index_publication"] as const

const statusKeys = [
  "schema_version", "status", "process_state", "core", "optional_runtimes", "capabilities", "error", "repair_progress",
  "indexed_documents", "searchable_documents", "partial_documents", "visible_epoch", "failed_retryable", "failed_permanent",
  "recovery_queue_depth", "ocr_queue_depth", "ocr_jobs_queued", "ocr_page_budget_blocked", "ocr_remediation",
  "ocr_language_unavailable", "ocr_language_remediation", "embedding_queue_depth", "entity_mentions", "import_tasks_queued",
  "import_tasks_recoverable", "import_tasks_cancelled", "import_scan_scopes", "import_scan_errors", "query_latency",
  "latest_import_scan", "active_profile", "index_health", "snapshot_present", "ipc",
]

export function isStatusReply(value: unknown): value is DaemonReply<StatusBody> {
  return isReply(value) && value.http_status === 200 && isStatusBody(value.body)
}

export function isDiagnosticsReply(value: unknown): value is DaemonReply<DiagnosticsBody> {
  return isReply(value) && value.http_status === 200 && isDiagnosticsBody(value.body)
}

export function isStatusBody(value: unknown): value is StatusBody {
  if (!isRecord(value) || !hasExactKeys(value, statusKeys)) return false
  if (value.schema_version !== "daemon.status.v3" || value.process_state !== "ready") return false
  if (!["initializing", "ok", "repairing", "degraded", "blocked"].includes(String(value.status))) return false
  if (!isCore(value.core) || !isRuntimes(value.optional_runtimes) || !isCapabilities(value.capabilities)) return false
  if (!healthStateMatches(value as unknown as StatusBody) || !capabilityMatrixMatches(value.core, value.optional_runtimes, value.capabilities)) return false
  if (!isServiceError(value.error, value.core)) return false
  if (!isRepairProgress(value.repair_progress, value.core.state)) return false
  if (!isIpc(value.ipc)) return false

  const storeKeys = [
    "indexed_documents", "searchable_documents", "partial_documents", "failed_retryable", "failed_permanent", "recovery_queue_depth",
    "ocr_queue_depth", "ocr_jobs_queued", "ocr_page_budget_blocked", "ocr_language_unavailable", "embedding_queue_depth", "entity_mentions",
    "import_tasks_queued", "import_tasks_recoverable", "import_tasks_cancelled", "import_scan_scopes", "import_scan_errors",
  ]
  if (!nullableSafeCount(value.visible_epoch) || !storeKeys.every((key) => nullableSafeCount(value[key]))) return false
  const storeReady = value.visible_epoch !== null
  if (!storeKeys.every((key) => (value[key] !== null) === storeReady)) return false
  if ((value.query_latency !== null) !== storeReady || (value.index_health !== null) !== storeReady || (value.active_profile !== null) !== storeReady || (value.snapshot_present !== null) !== storeReady) return false
  if (storeReady) {
    if (!isStatusLatency(value.query_latency) || !["empty", "building", "ready", "stale"].includes(String(value.index_health)) || value.active_profile !== "balanced" || typeof value.snapshot_present !== "boolean") return false
    if (!isRemediation(value.ocr_page_budget_blocked, value.ocr_remediation, "raise OCR max pages per document or skip oversized scanned PDFs")) return false
    if (!isRemediation(value.ocr_language_unavailable, value.ocr_language_remediation, "install requested OCR language packs or choose an installed OCR language")) return false
  } else if (value.ocr_remediation !== null || value.ocr_language_remediation !== null) {
    return false
  }
  return (value.latest_import_scan === null || (storeReady && isLatestImportScan(value.latest_import_scan)))
}

export function isDiagnosticsBody(value: unknown): value is DiagnosticsBody {
  if (!isRecord(value) || !hasExactKeys(value, [
    "schema_version", "privacy_boundary", "contains_raw_resume_text", "contains_queries", "contains_resume_paths",
    "contains_candidate_results", "contains_snippet_text", "visible_epoch", "evidence_lane", "evidence_status",
    "process_state", "core", "optional_runtimes", "capabilities", "repair_progress", "error", "metrics", "error_counts",
  ])) return false
  if (value.schema_version !== "resume-ir.diagnostics.v4" || value.privacy_boundary !== "redacted_local_aggregate" || value.evidence_lane !== "gui_manual" || value.evidence_status !== "unaccepted" || value.process_state !== "ready") return false
  if (![value.contains_raw_resume_text, value.contains_queries, value.contains_resume_paths, value.contains_candidate_results, value.contains_snippet_text].every((flag) => flag === false)) return false
  if (!nullableSafeCount(value.visible_epoch) || !isCore(value.core) || !isRuntimes(value.optional_runtimes) || !isCapabilities(value.capabilities)) return false
  if (!capabilityMatrixMatches(value.core, value.optional_runtimes, value.capabilities) || !isServiceError(value.error, value.core) || !isRepairProgress(value.repair_progress, value.core.state)) return false
  return isDiagnosticsMetrics(value.metrics, value.visible_epoch !== null)
    && isDiagnosticsErrors(value.error_counts, value.visible_epoch !== null)
}

function isCore(value: unknown): value is StatusBody["core"] {
  if (!isRecord(value) || !hasExactKeys(value, ["state", "reason"]) || !coreStates.includes(value.state as typeof coreStates[number])) return false
  if (value.state === "ready") return value.reason === null
  if (value.state === "initializing") return value.reason === "metadata_initializing"
  if (value.state === "repairing") return value.reason === "migration_rebuild" || value.reason === "artifact_unavailable"
  return ["artifact_unavailable", "source_unavailable", "runtime_invariant", "unsupported_store_schema", "metadata_unavailable"].includes(String(value.reason))
}

function isRuntime(value: unknown): value is OptionalRuntimeStatus {
  if (!isRecord(value) || !hasExactKeys(value, ["state", "reason"]) || !runtimeStates.includes(value.state as typeof runtimeStates[number])) return false
  return value.state === "unavailable"
    ? runtimeReasons.includes(value.reason as typeof runtimeReasons[number])
    : value.reason === null
}

function isRuntimes(value: unknown): value is StatusBody["optional_runtimes"] {
  return isRecord(value) && hasExactKeys(value, ["embedding", "ocr", "classifier"])
    && isRuntime(value.embedding) && isRuntime(value.ocr) && isRuntime(value.classifier)
}

function isCapabilities(value: unknown): value is StatusBody["capabilities"] {
  return isRecord(value) && hasExactKeys(value, [...capabilityNames])
    && capabilityNames.every((name) => isCapability(value[name]))
}

function isCapability(value: unknown): value is CapabilityStatus {
  if (!isRecord(value) || !hasExactKeys(value, ["state", "reason"]) || !capabilityStates.includes(value.state as typeof capabilityStates[number])) return false
  if (value.state === "available") return value.reason === null
  if (value.state === "initializing") return value.reason === "core_initializing"
  return capabilityReasons.includes(value.reason as typeof capabilityReasons[number])
}

function capabilityMatrixMatches(core: StatusBody["core"], runtimes: StatusBody["optional_runtimes"], capabilities: StatusBody["capabilities"]): boolean {
  if (core.state === "initializing" || core.state === "repairing") return capabilityNames.every((name) => capabilityIs(capabilities[name], "initializing", "core_initializing"))
  if (core.state === "degraded" || core.state === "blocked") return capabilityNames.every((name) => capabilityIs(capabilities[name], "blocked", "core_blocked"))
  if (!capabilityIs(capabilities.keyword_search, "available", null) || !capabilityIs(capabilities.detail, "available", null)) return false
  const embedding = runtimes.embedding.state === "available"
  const classifier = runtimes.classifier.state === "available"
  const ocr = runtimes.ocr.state === "available"
  return capabilityIs(capabilities.semantic_search, embedding ? "available" : "unavailable", embedding ? null : "embedding_unavailable")
    && capabilityIs(capabilities.hybrid_search, embedding ? "available" : "degraded", embedding ? null : "embedding_unavailable")
    && capabilityIs(capabilities.text_import, classifier && embedding ? "available" : "unavailable", !classifier ? "classifier_unavailable" : !embedding ? "embedding_unavailable" : null)
    && capabilityIs(capabilities.ocr_import, classifier && embedding && ocr ? "available" : "unavailable", !classifier ? "classifier_unavailable" : !embedding ? "embedding_unavailable" : !ocr ? "ocr_unavailable" : null)
    && capabilityIs(capabilities.index_publication, embedding ? "available" : "unavailable", embedding ? null : "embedding_unavailable")
}

function capabilityIs(value: CapabilityStatus, state: CapabilityStatus["state"], reason: CapabilityReason | null): boolean {
  return value.state === state && value.reason === reason
}

function isServiceError(value: unknown, core: StatusBody["core"]): boolean {
  if (core.state === "ready") return value === null
  if (!isRecord(value) || !hasExactKeys(value, ["code", "action", "capability", "reason"]) || value.capability !== null || value.reason !== core.reason) return false
  if (core.state === "initializing" || core.state === "repairing") return value.code === "SERVICE_INITIALIZING" && value.action === "wait_for_service"
  return value.code === "SERVICE_BLOCKED" && value.action === (core.state === "degraded" ? "retry" : "repair_required")
}

function healthStateMatches(value: StatusBody): boolean {
  const expected = { initializing: "initializing", ready: "ok", repairing: "repairing", degraded: "degraded", blocked: "blocked" }[value.core.state]
  return value.status === expected
}

function isRepairProgress(value: unknown, core: StatusBody["core"]["state"]): boolean {
  if (value === null) return core !== "repairing"
  if (!isRecord(value) || !hasExactKeys(value, ["phase", "attempt", "max_attempts", "retry_after_ms", "last_error_kind"])) return false
  if (!["queued", "migration_rebuild", "source_unavailable", "rebuilding", "retry_wait", "blocked"].includes(String(value.phase))) return false
  if (![value.attempt, value.max_attempts, value.retry_after_ms].every(nullableSafeCount)) return false
  if (value.attempt !== null && Number(value.attempt) > 5) return false
  if (value.max_attempts !== null && value.max_attempts !== 5) return false
  if (value.retry_after_ms !== null && Number(value.retry_after_ms) > 60_000) return false
  return value.last_error_kind === null || ["fulltext_publication_busy", "fulltext_failure", "vector_publication_busy", "vector_failure", "metadata_failure", "interrupted"].includes(String(value.last_error_kind))
}

function isStatusLatency(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, ["sample_count", "p50_ms", "p95_ms", "p99_ms", "last_result_count", "raw_queries"])
    && safeCount(value.sample_count) && nullableSafeCount(value.last_result_count) && value.raw_queries === "<redacted>"
    && latencyValuesValid(value.p50_ms, value.p95_ms, value.p99_ms)
}

function isLatestImportScan(value: unknown): boolean {
  if (!isRecord(value) || !hasExactKeys(value, ["scan_profile", "files_discovered", "ignored_entries", "scan_errors", "searchable_documents", "ocr_required_documents", "ocr_jobs_queued", "failed_documents", "deleted_documents", "scan_budget_observed", "scan_budget_limit", "scan_budget_exhausted"])) return false
  return ["explicit", "discovery"].includes(String(value.scan_profile))
    && ["files_discovered", "ignored_entries", "scan_errors", "searchable_documents", "ocr_required_documents", "ocr_jobs_queued", "failed_documents", "deleted_documents"].every((key) => safeCount(value[key]))
    && nullableSafeCount(value.scan_budget_observed) && nullableSafeCount(value.scan_budget_limit) && typeof value.scan_budget_exhausted === "boolean"
}

function isIpc(value: unknown): boolean {
  return isRecord(value) && hasExactKeys(value, ["accepted", "completed", "client_disconnect", "request_failure", "response_failure"])
    && Object.values(value).every(safeCount)
}

function isDiagnosticsMetrics(value: unknown, storeReady: boolean): boolean {
  if (!isRecord(value) || !hasExactKeys(value, ["ipc", "indexed_documents", "searchable_documents", "partial_documents", "ocr_queue_depth", "embedding_queue_depth", "recovery_queue_depth", "import_tasks_queued", "import_tasks_recoverable", "import_tasks_cancelled", "query_latency"]) || !isIpc(value.ipc)) return false
  const counts = ["indexed_documents", "searchable_documents", "partial_documents", "ocr_queue_depth", "embedding_queue_depth", "recovery_queue_depth", "import_tasks_queued", "import_tasks_recoverable", "import_tasks_cancelled"]
  if (!counts.every((key) => nullableSafeCount(value[key]) && (value[key] !== null) === storeReady)) return false
  if (!isRecord(value.query_latency) || !hasExactKeys(value.query_latency, ["sample_count", "p50_ms", "p95_ms", "p99_ms", "last_result_count"])) return false
  const latency = value.query_latency
  if (!nullableSafeCount(latency.sample_count) || (latency.sample_count !== null) !== storeReady || !nullableSafeCount(latency.last_result_count)) return false
  return latencyValuesValid(latency.p50_ms, latency.p95_ms, latency.p99_ms)
}

function isDiagnosticsErrors(value: unknown, storeReady: boolean): boolean {
  if (!isRecord(value) || !hasExactKeys(value, ["failed_retryable", "failed_permanent", "import_scan_errors", "ocr_page_budget_blocked", "ocr_language_unavailable", "scan_error_buckets"])) return false
  const counts = ["failed_retryable", "failed_permanent", "import_scan_errors", "ocr_page_budget_blocked", "ocr_language_unavailable"]
  if (!counts.every((key) => nullableSafeCount(value[key]) && (value[key] !== null) === storeReady)) return false
  if (!Array.isArray(value.scan_error_buckets) || value.scan_error_buckets.length > 16) return false
  return value.scan_error_buckets.every((bucket) => isRecord(bucket) && hasExactKeys(bucket, ["class", "operation", "count"])
    && ["permission_denied", "source_unavailable", "locked_or_unreadable", "io"].includes(String(bucket.class))
    && ["normalize_path", "read_directory", "read_metadata", "fingerprint"].includes(String(bucket.operation)) && safeCount(bucket.count))
}

function isRemediation(count: unknown, value: unknown, remediation: string): boolean {
  return count === 0 ? value === "none" : safeCount(count) && Number(count) > 0 && value === remediation
}

function latencyValuesValid(p50: unknown, p95: unknown, p99: unknown): boolean {
  return [p50, p95, p99].every((value) => value === null || (typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 3_600_000))
}

function isReply(value: unknown): value is { http_status: number; body: unknown } {
  return isRecord(value) && hasExactKeys(value, ["http_status", "body"])
    && Number.isInteger(value.http_status) && Number(value.http_status) >= 100 && Number(value.http_status) <= 599
}

function nullableSafeCount(value: unknown): boolean { return value === null || safeCount(value) }
function safeCount(value: unknown): boolean { return Number.isSafeInteger(value) && Number(value) >= 0 }
function isRecord(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value) }

function hasExactKeys(value: Record<string, unknown>, required: string[], optional: string[] = []): boolean {
  const allowed = new Set([...required, ...optional])
  const actual = Object.keys(value)
  return required.every((key) => Object.prototype.hasOwnProperty.call(value, key)) && actual.every((key) => allowed.has(key))
}
