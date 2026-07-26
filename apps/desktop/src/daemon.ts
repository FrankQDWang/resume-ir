import { invoke } from "@tauri-apps/api/core"

import { isDiagnosticsReply, isStatusReply } from "./daemon-contract"
import { isDaemonLifecycleSnapshot, type DaemonLifecycleSnapshot } from "./runtime-state"

export interface DaemonReply<T> {
  http_status: number
  body: T
}

export interface BridgeError {
  code: string
  message: string
}

export type BridgeFailureKind = "overload" | "unavailable" | "stale_selection" | "selection_missing" | "error"

export interface SearchSelection {
  doc_id: string
  version_id: string
  visible_epoch: number
}

export interface SearchHit {
  rank: number
  selection: SearchSelection
  file_name: string
  snippet: string
}

export type CoreState = "initializing" | "ready" | "repairing" | "degraded" | "blocked"
export type CoreReason = "metadata_initializing" | "migration_rebuild" | "artifact_unavailable" | "source_unavailable" | "runtime_invariant" | "unsupported_store_schema" | "metadata_unavailable"
export type OptionalRuntimeState = "initializing" | "available" | "unavailable"
export type OptionalRuntimeReason = "missing" | "invalid" | "start_failed" | "not_configured"
export type CapabilityState = "initializing" | "available" | "degraded" | "unavailable" | "blocked"
export type CapabilityReason = "core_initializing" | "core_blocked" | "embedding_unavailable" | "ocr_unavailable" | "classifier_unavailable"
export type CapabilityName = "keyword_search" | "detail" | "semantic_search" | "hybrid_search" | "text_import" | "ocr_import" | "index_publication"

export interface OptionalRuntimeStatus {
  state: OptionalRuntimeState
  reason: OptionalRuntimeReason | null
}

export interface CapabilityStatus {
  state: CapabilityState
  reason: CapabilityReason | null
}

export interface RepairProgress {
  phase: "queued" | "migration_rebuild" | "source_unavailable" | "rebuilding" | "retry_wait" | "blocked"
  attempt: number | null
  max_attempts: number | null
  retry_after_ms: number | null
  last_error_kind: "fulltext_publication_busy" | "fulltext_failure" | "vector_publication_busy" | "vector_failure" | "metadata_failure" | "interrupted" | null
}

export interface DaemonServiceError {
  code: "UNAUTHORIZED" | "BAD_REQUEST" | "CONFLICT" | "NOT_FOUND" | "STALE_SELECTION" | "RESPONSE_TOO_LARGE" | "LIMIT_EXCEEDED" | "SEMANTIC_DISABLED" | "REPAIRING" | "METADATA_UNAVAILABLE" | "QUERY_SERVICE_UNAVAILABLE" | "OVERLOADED" | "INTERNAL" | "SERVICE_INITIALIZING" | "SERVICE_BLOCKED" | "CAPABILITY_UNAVAILABLE"
  action: "authenticate" | "correct_request" | "refresh_search" | "reduce_page_size" | "select_supported_mode" | "wait_for_repair" | "wait_for_service" | "retry" | "repair_required"
  retry_after_ms?: number
  capability: CapabilityName | null
  reason: CoreReason | CapabilityReason | null
}

export interface DaemonServiceErrorBody {
  schema_version: "resume-ir.error.v2"
  request_id?: string
  status: "error"
  error: DaemonServiceError
}

export type DaemonFailureBody = DaemonServiceErrorBody

export interface IpcMetrics {
  accepted: number
  completed: number
  client_disconnect: number
  request_failure: number
  response_failure: number
}

export interface StatusBody {
  schema_version: "daemon.status.v3"
  status: "initializing" | "ok" | "repairing" | "degraded" | "blocked"
  process_state: "ready"
  core: {
    state: CoreState
    reason: CoreReason | null
  }
  optional_runtimes: {
    embedding: OptionalRuntimeStatus
    ocr: OptionalRuntimeStatus
    classifier: OptionalRuntimeStatus
  }
  capabilities: {
    keyword_search: CapabilityStatus
    detail: CapabilityStatus
    semantic_search: CapabilityStatus
    hybrid_search: CapabilityStatus
    text_import: CapabilityStatus
    ocr_import: CapabilityStatus
    index_publication: CapabilityStatus
  }
  repair_progress: RepairProgress | null
  error: DaemonServiceError | null
  indexed_documents: number | null
  searchable_documents: number | null
  partial_documents: number | null
  visible_epoch: number | null
  failed_retryable: number | null
  failed_permanent: number | null
  recovery_queue_depth: number | null
  ocr_queue_depth: number | null
  ocr_jobs_queued: number | null
  ocr_page_budget_blocked: number | null
  ocr_remediation: "none" | "raise OCR max pages per document or skip oversized scanned PDFs" | null
  ocr_language_unavailable: number | null
  ocr_language_remediation: "none" | "install requested OCR language packs or choose an installed OCR language" | null
  embedding_queue_depth: number | null
  entity_mentions: number | null
  import_tasks_queued: number | null
  import_tasks_recoverable: number | null
  import_tasks_cancelled: number | null
  import_scan_scopes: number | null
  import_scan_errors: number | null
  query_latency: null | {
    sample_count: number
    p50_ms: number | null
    p95_ms: number | null
    p99_ms: number | null
    last_result_count: number | null
    raw_queries: "<redacted>"
  }
  index_health: "empty" | "building" | "ready" | "stale" | null
  latest_import_scan: null | {
    scan_profile: "explicit" | "discovery"
    files_discovered: number
    ignored_entries: number
    scan_errors: number
    searchable_documents: number
    ocr_required_documents: number
    ocr_jobs_queued: number
    failed_documents: number
    deleted_documents: number
    scan_budget_observed: number | null
    scan_budget_limit: number | null
    scan_budget_exhausted: boolean
  }
  active_profile: "balanced" | null
  snapshot_present: boolean | null
  ipc: IpcMetrics
}

export function daemonHealth(reply: DaemonReply<StatusBody | DaemonFailureBody>): "ok" | "degraded" {
  return reply.http_status === 200
    && reply.body.schema_version === "daemon.status.v3"
    && reply.body.status === "ok"
    && reply.body.core.state === "ready"
    ? "ok"
    : "degraded"
}

export interface SearchRequestBody {
  schema_version: "resume-ir.ipc-request.v3"
  request_id: string
  client_capability: "interactive_gui"
  deadline_ms: number
  cancel_token?: string
  payload: {
    query: string
    mode: "fulltext" | "semantic" | "hybrid"
    top_k: number
    filters: {
      degree_min?: "associate" | "bachelor" | "master" | "doctorate"
      skills_any?: string[]
      locations_any?: string[]
      years_experience_min?: number
    }
  }
}

export interface SearchSuccessBody {
  schema_version: "resume-ir.search-response.v3"
  request_id: string
  status: "ok" | "cancelled"
  visible_epoch: number
  query_mode: "keyword" | "field_filter" | "hybrid" | "semantic"
  partial: boolean
  partial_reasons: Array<"search_index_not_ready" | "deadline_exceeded" | "embedding_runtime_unavailable">
  latency_ms: number
  result_count: number
  results: SearchHit[]
}

export type SearchBody = SearchSuccessBody

export type SearchOutcome = "complete" | "partial" | "empty" | "overload" | "cancelled" | "error"

export function searchDeadlineMs(mode: "keyword" | "field" | "hybrid" | "semantic"): number {
  return mode === "semantic" || mode === "hybrid" ? 30000 : 1500
}

export function searchOutcome(reply: DaemonReply<SearchBody | DaemonFailureBody>): SearchOutcome {
  if (reply.body.schema_version === "resume-ir.error.v2") return reply.body.error.code === "OVERLOADED" ? "overload" : "error"
  if (reply.body.status === "cancelled") return "cancelled"
  if (reply.http_status < 200 || reply.http_status >= 300) return "error"
  if (reply.body.partial) return "partial"
  return reply.body.results.length === 0 ? "empty" : "complete"
}

export interface DetailBody {
  schema_version: "resume-ir.detail-response.v3"
  request_id: string
  selection: SearchSelection
  status: "ok"
  document: {
    source_byte_size: number
    parse_version: string
    schema_version: string
    language_set: string[]
    page_count: number | null
    quality_score: number | null
    fields_truncated: boolean
    fields: Array<{ type: string; value: string; confidence: number }>
    snippet: string
  }
  limits: {
    max_fields: number
    max_response_bytes: number
  }
}

export interface DetailHydrateBody {
  schema_version: "resume-ir.detail-hydrate-response.v3"
  request_id: string
  selection: SearchSelection
  status: "ok"
  document: {
    body_page: {
      encoding: "utf-8"
      offset_bytes: number
      next_offset_bytes: number
      total_bytes: number
      complete: boolean
      text: string
    }
  }
  privacy: {
    local_authenticated_only: true
    public_output_allowed: false
  }
  limits: {
    max_body_page_bytes: number
    max_response_bytes: number
  }
}

export interface SelectedImportRoot {
  root_handle: string
  display_label: string
}

export interface ManagedRoot extends SelectedImportRoot {
  availability: "available" | "unavailable"
}

export interface ManagedRoots {
  schema_version: "resume-ir.desktop-managed-roots.v1"
  limit: 16
  roots: ManagedRoot[]
}

export interface ImportBody {
  schema_version: "daemon.import.v1"
  status: "accepted"
  accepted_roots: number
  new_tasks: number
  scan_profile: "explicit"
  scan_file_limit: number | null
}

export type ManagedRootScanOutcome = "queued" | "pending" | "active" | "error"

export function managedRootScanOutcome(reply: DaemonReply<ImportBody | DaemonFailureBody>): ManagedRootScanOutcome {
  if (reply.body.schema_version === "resume-ir.error.v2") {
    return reply.http_status === 409 && reply.body.error.code === "CONFLICT" ? "active" : "error"
  }
  if (reply.http_status < 200 || reply.http_status >= 300) return "error"
  return reply.body.new_tasks === 1 ? "queued" : "pending"
}

export type ManagedRootControlAction = "inspect" | "pause" | "resume"
export type ManagedRootControlOutcome = "unmanaged" | "active" | "paused" | "error"

export interface ManagedRootControlBody {
  schema_version: "daemon.import_root_control.v1"
  status: "active" | "paused"
  changed: boolean
  task_cancel_requested: boolean
  catch_up_queued: boolean
}

export function managedRootControlOutcome(reply: DaemonReply<ManagedRootControlBody | DaemonFailureBody>): ManagedRootControlOutcome {
  if (reply.body.schema_version === "resume-ir.error.v2") {
    return reply.http_status === 404 && reply.body.error.code === "NOT_FOUND" ? "unmanaged" : "error"
  }
  if (reply.http_status < 200 || reply.http_status >= 300) return "error"
  return reply.body.status
}

export interface DiagnosticsBody {
  schema_version: "resume-ir.diagnostics.v4"
  privacy_boundary: "redacted_local_aggregate"
  evidence_lane: "gui_manual"
  evidence_status: "unaccepted"
  contains_raw_resume_text: false
  contains_queries: false
  contains_resume_paths: false
  contains_candidate_results: false
  contains_snippet_text: false
  visible_epoch: number | null
  process_state: "ready"
  core: {
    state: CoreState
    reason: CoreReason | null
  }
  optional_runtimes: {
    embedding: OptionalRuntimeStatus
    ocr: OptionalRuntimeStatus
    classifier: OptionalRuntimeStatus
  }
  capabilities: {
    keyword_search: CapabilityStatus
    detail: CapabilityStatus
    semantic_search: CapabilityStatus
    hybrid_search: CapabilityStatus
    text_import: CapabilityStatus
    ocr_import: CapabilityStatus
    index_publication: CapabilityStatus
  }
  repair_progress: RepairProgress | null
  error: DaemonServiceError | null
  metrics: {
    ipc: IpcMetrics
    indexed_documents: number | null
    searchable_documents: number | null
    partial_documents: number | null
    ocr_queue_depth: number | null
    embedding_queue_depth: number | null
    recovery_queue_depth: number | null
    import_tasks_queued: number | null
    import_tasks_recoverable: number | null
    import_tasks_cancelled: number | null
    query_latency: null | {
      sample_count: number | null
      p50_ms: number | null
      p95_ms: number | null
      p99_ms: number | null
      last_result_count: number | null
    }
  }
  error_counts: {
    failed_retryable: number | null
    failed_permanent: number | null
    import_scan_errors: number | null
    ocr_page_budget_blocked: number | null
    ocr_language_unavailable: number | null
    scan_error_buckets: Array<{ class: string; operation: string; count: number }>
  }
}

export interface DiagnosticsExportReceipt {
  status: "saved"
  file_label: string
}

export interface SearchCancelBody {
  schema_version: "resume-ir.search-cancel-response.v1"
  request_id: string
  status: "cancelled" | "cancel_requested" | "complete"
}

export async function readStatus(): Promise<DaemonReply<StatusBody>> {
  const reply = await invoke<unknown>("daemon_request", {
    request: { operation: "status" },
  })
  if (!isStatusReply(reply)) throw contractFailure("daemon status v3 合同无效")
  return reply
}

export async function readDiagnostics(): Promise<DaemonReply<DiagnosticsBody>> {
  const reply = await invoke<unknown>("daemon_request", {
    request: { operation: "diagnostics" },
  })
  if (!isDiagnosticsReply(reply)) throw contractFailure("daemon diagnostics v4 合同无效")
  return reply
}

function contractFailure(message: string): BridgeError {
  return { code: "daemon_contract", message }
}

export async function searchResumes(body: SearchRequestBody): Promise<DaemonReply<SearchBody | DaemonFailureBody>> {
  return invoke<DaemonReply<SearchBody | DaemonFailureBody>>("daemon_request", {
    request: { operation: "search", body },
  })
}

export async function readDetail(requestId: string, selection: SearchSelection): Promise<DaemonReply<DetailBody | DaemonFailureBody>> {
  return invoke<DaemonReply<DetailBody | DaemonFailureBody>>("daemon_request", {
    request: {
      operation: "detail",
      body: {
        schema_version: "resume-ir.detail-request.v3",
        request_id: requestId,
        selection,
      },
    },
  })
}

export async function requestSearchCancel(requestId: string, cancelToken: string): Promise<DaemonReply<SearchCancelBody | DaemonFailureBody>> {
  return invoke<DaemonReply<SearchCancelBody | DaemonFailureBody>>("daemon_request", {
    request: {
      operation: "cancel",
      body: {
        schema_version: "resume-ir.search-cancel-request.v1",
        request_id: requestId,
        cancel_token: cancelToken,
      },
    },
  })
}

export async function getDaemonLifecycle(): Promise<DaemonLifecycleSnapshot> {
  const snapshot = await invoke<unknown>("get_daemon_lifecycle")
  if (!isDaemonLifecycleSnapshot(snapshot)) throw contractFailure("desktop lifecycle v2 合同无效")
  return snapshot
}

export async function retryDaemon(): Promise<DaemonLifecycleSnapshot> {
  const snapshot = await invoke<unknown>("retry_daemon")
  if (!isDaemonLifecycleSnapshot(snapshot)) throw contractFailure("desktop lifecycle v2 合同无效")
  return snapshot
}

export async function hydrateDetail(requestId: string, selection: SearchSelection, bodyOffsetBytes: number): Promise<DaemonReply<DetailHydrateBody | DaemonFailureBody>> {
  return invoke<DaemonReply<DetailHydrateBody | DaemonFailureBody>>("daemon_request", {
    request: {
      operation: "hydrate",
      body: {
        schema_version: "resume-ir.detail-hydrate-request.v3",
        request_id: requestId,
        selection,
        body_offset_bytes: bodyOffsetBytes,
        body_limit_bytes: 32 * 1024,
      },
    },
  })
}

export async function selectImportRoot(): Promise<SelectedImportRoot | null> {
  return invoke<SelectedImportRoot | null>("select_import_root")
}

export async function listManagedRoots(): Promise<ManagedRoots> {
  return invoke<ManagedRoots>("list_managed_roots")
}

export async function importSelectedRoot(rootHandle: string): Promise<DaemonReply<ImportBody | DaemonFailureBody>> {
  return invoke<DaemonReply<ImportBody | DaemonFailureBody>>("import_selected_root", { request: { root_handle: rootHandle } })
}

export async function reauthorizeManagedRoot(rootHandle: string): Promise<SelectedImportRoot | null> {
  return invoke<SelectedImportRoot | null>("reauthorize_managed_root", { request: { root_handle: rootHandle } })
}

export async function rescanManagedRoot(rootHandle: string): Promise<DaemonReply<ImportBody | DaemonFailureBody>> {
  return importSelectedRoot(rootHandle)
}

export async function controlManagedRoot(rootHandle: string, action: ManagedRootControlAction): Promise<DaemonReply<ManagedRootControlBody | DaemonFailureBody>> {
  return invoke<DaemonReply<ManagedRootControlBody | DaemonFailureBody>>("daemon_request", {
    request: { operation: "root_control", body: { root_handle: rootHandle, action } },
  })
}

export type ManagedRootRecoveryFailure = "overload" | "mismatch" | "unavailable" | "error"

export function managedRootRecoveryFailure(error: unknown): ManagedRootRecoveryFailure {
  const projected = bridgeError(error)
  if (projected.code === "bridge_overloaded") return "overload"
  if (projected.code === "managed_root_mismatch") return "mismatch"
  if (projected.code === "import_root_unavailable" || projected.code === "import_root_unreadable") return "unavailable"
  return "error"
}

export async function exportDiagnostics(): Promise<DiagnosticsExportReceipt | null> {
  return invoke<DiagnosticsExportReceipt | null>("export_diagnostics")
}

export function bridgeError(error: unknown): BridgeError {
  if (typeof error === "object" && error !== null && "code" in error && "message" in error) {
    return { code: String(error.code), message: String(error.message) }
  }
  return { code: "bridge_error", message: "桌面桥接请求失败" }
}

export function bridgeFailureKind(error: unknown): BridgeFailureKind {
  const projected = bridgeError(error)
  if (projected.code === "bridge_overloaded") return "overload"
  if (
    projected.code === "daemon_unavailable"
    || projected.code === "daemon_generation_changed"
    || projected.code === "REPAIRING"
    || projected.code === "METADATA_UNAVAILABLE"
    || projected.code === "QUERY_SERVICE_UNAVAILABLE"
    || projected.code === "SERVICE_INITIALIZING"
    || projected.code === "SERVICE_BLOCKED"
    || projected.code === "CAPABILITY_UNAVAILABLE"
  ) return "unavailable"
  if (projected.code === "STALE_SELECTION") return "stale_selection"
  if (projected.code === "NOT_FOUND") return "selection_missing"
  return "error"
}

export function sameSearchSelection(left: SearchSelection, right: SearchSelection): boolean {
  return left.doc_id === right.doc_id
    && left.version_id === right.version_id
    && left.visible_epoch === right.visible_epoch
}
