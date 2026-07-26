import { clearMocks, mockIPC } from "@tauri-apps/api/mocks"
import { beforeEach, describe, expect, it } from "vitest"

import readyStatusFixture from "../src-tauri/tests/fixtures/daemon-status-v3-ready.json"
import artifactBlockedStatusFixture from "../src-tauri/tests/fixtures/daemon-status-v3-artifact-blocked.json"
import healthCombinationsFixture from "../../../crates/daemon-contract/tests/fixtures/health-combinations-v1.json"
import {
  bridgeError,
  bridgeFailureKind,
  controlManagedRoot,
  daemonHealth,
  exportDiagnostics,
  getDaemonLifecycle,
  hydrateDetail,
  importSelectedRoot,
  managedRootControlOutcome,
  managedRootScanOutcome,
  readDetail,
  readDiagnostics,
  readStatus,
  requestSearchCancel,
  retryDaemon,
  searchDeadlineMs,
  searchOutcome,
  searchResumes,
  sameSearchSelection,
  type DaemonServiceErrorBody,
  type DiagnosticsBody,
  type StatusBody,
} from "./daemon"
import type { DaemonLifecycleSnapshot } from "./runtime-state"

interface IpcCall { command: string; payload: unknown }
let ipcCalls: IpcCall[] = []

if (typeof window === "undefined") {
  Object.defineProperty(globalThis, "window", { configurable: true, value: globalThis })
}

beforeEach(() => {
  clearMocks()
  ipcCalls = []
})

function mockReply(reply: unknown): void {
  mockIPC((command, payload) => {
    ipcCalls.push({ command, payload })
    return reply
  })
}

function readyStatus(): StatusBody {
  return structuredClone(readyStatusFixture) as StatusBody
}

function runningLifecycle(): DaemonLifecycleSnapshot {
  return {
    schema_version: "resume-ir.desktop-daemon-lifecycle.v2",
    state: "running",
    transition_reason: "control_plane_ready",
    generation: 3,
    automatic_restart_attempt: 0,
    automatic_restart_limit: 5,
    retry_after_ms: null,
    heartbeat_failures: 0,
    last_exit: null,
  }
}

function diagnostics(): DiagnosticsBody {
  const status = readyStatus()
  return {
    schema_version: "resume-ir.diagnostics.v4",
    privacy_boundary: "redacted_local_aggregate",
    evidence_lane: "gui_manual",
    evidence_status: "unaccepted",
    contains_raw_resume_text: false,
    contains_queries: false,
    contains_resume_paths: false,
    contains_candidate_results: false,
    contains_snippet_text: false,
    visible_epoch: 7,
    process_state: "ready",
    core: status.core,
    optional_runtimes: status.optional_runtimes,
    capabilities: status.capabilities,
    repair_progress: null,
    error: null,
    metrics: {
      ipc: status.ipc,
      indexed_documents: 4,
      searchable_documents: 3,
      partial_documents: 1,
      ocr_queue_depth: 0,
      embedding_queue_depth: 0,
      recovery_queue_depth: 0,
      import_tasks_queued: 0,
      import_tasks_recoverable: 0,
      import_tasks_cancelled: 0,
      query_latency: { sample_count: 1, p50_ms: 2, p95_ms: 3, p99_ms: 4, last_result_count: 1 },
    },
    error_counts: {
      failed_retryable: 0,
      failed_permanent: 0,
      import_scan_errors: 0,
      ocr_page_budget_blocked: 0,
      ocr_language_unavailable: 0,
      scan_error_buckets: [],
    },
  }
}

const badRequest: DaemonServiceErrorBody = {
  schema_version: "resume-ir.error.v2",
  request_id: "request-1",
  status: "error",
  error: { code: "BAD_REQUEST", action: "correct_request", capability: null, reason: null },
}

describe("desktop bridge errors", () => {
  it("keeps bounded native errors and classifies hard-cut service failures", () => {
    expect(bridgeError({ code: "daemon_unavailable", message: "本地 daemon 暂时不可用" })).toEqual({ code: "daemon_unavailable", message: "本地 daemon 暂时不可用" })
    expect(bridgeError(new Error("/private/local/path/ipc.auth"))).toEqual({ code: "bridge_error", message: "桌面桥接请求失败" })
    for (const code of ["daemon_unavailable", "daemon_generation_changed", "REPAIRING", "METADATA_UNAVAILABLE", "QUERY_SERVICE_UNAVAILABLE", "SERVICE_INITIALIZING", "SERVICE_BLOCKED", "CAPABILITY_UNAVAILABLE"]) {
      expect(bridgeFailureKind({ code, message: "bounded" })).toBe("unavailable")
    }
    expect(bridgeFailureKind({ code: "STALE_SELECTION", message: "bounded" })).toBe("stale_selection")
    expect(bridgeFailureKind({ code: "NOT_FOUND", message: "bounded" })).toBe("selection_missing")
  })
})

describe("strict control-plane contracts", () => {
  it("consumes the shared status v3 fixture", async () => {
    const status = readyStatus()
    mockReply({ http_status: 200, body: status })
    await expect(readStatus()).resolves.toEqual({ http_status: 200, body: status })
    expect(daemonHealth({ http_status: 200, body: status })).toBe("ok")
  })

  it("rejects old status, unknown fields, missing nullable fields, and impossible capabilities", async () => {
    const old = { ...readyStatus(), schema_version: "daemon.status.v2" }
    mockReply({ http_status: 200, body: old })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })

    const unknown = { ...readyStatus(), private_debug: true }
    mockReply({ http_status: 200, body: unknown })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })

    const missing = readyStatus() as unknown as Record<string, unknown>
    delete missing.repair_progress
    mockReply({ http_status: 200, body: missing })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })

    const impossible = readyStatus()
    impossible.capabilities.semantic_search = { state: "available", reason: null }
    impossible.optional_runtimes.embedding = { state: "unavailable", reason: "invalid" }
    mockReply({ http_status: 200, body: impossible })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
  })

  it("accepts the embedding-only degradation matrix and rejects an import escape", async () => {
    const degraded = readyStatus()
    degraded.optional_runtimes.embedding = { state: "unavailable", reason: "invalid" }
    degraded.capabilities.semantic_search = { state: "unavailable", reason: "embedding_unavailable" }
    degraded.capabilities.hybrid_search = { state: "degraded", reason: "embedding_unavailable" }
    degraded.capabilities.text_import = { state: "unavailable", reason: "embedding_unavailable" }
    degraded.capabilities.ocr_import = { state: "unavailable", reason: "embedding_unavailable" }
    degraded.capabilities.index_publication = { state: "unavailable", reason: "embedding_unavailable" }
    mockReply({ http_status: 200, body: degraded })
    await expect(readStatus()).resolves.toEqual({ http_status: 200, body: degraded })

    degraded.capabilities.text_import = { state: "available", reason: null }
    mockReply({ http_status: 200, body: degraded })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
  })

  it("keeps index publication available when only the classifier is unavailable", async () => {
    const degraded = readyStatus()
    degraded.optional_runtimes.classifier = { state: "unavailable", reason: "not_configured" }
    degraded.capabilities.text_import = { state: "unavailable", reason: "classifier_unavailable" }
    degraded.capabilities.ocr_import = { state: "unavailable", reason: "classifier_unavailable" }
    mockReply({ http_status: 200, body: degraded })
    await expect(readStatus()).resolves.toEqual({ http_status: 200, body: degraded })

    degraded.capabilities.index_publication = { state: "unavailable", reason: "classifier_unavailable" }
    mockReply({ http_status: 200, body: degraded })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
  })

  it("accepts every shared ready-runtime capability combination", async () => {
    expect(healthCombinationsFixture.schema_version).toBe("resume-ir.daemon-health-conformance.v1")
    expect(healthCombinationsFixture.cases).toHaveLength(8)
    for (const testCase of healthCombinationsFixture.cases) {
      const status = readyStatus()
      for (const runtime of ["embedding", "ocr", "classifier"] as const) {
        status.optional_runtimes[runtime] = testCase.runtime_availability[runtime]
          ? { state: "available", reason: null }
          : { state: "unavailable", reason: "not_configured" }
      }
      status.capabilities = structuredClone(testCase.capabilities) as StatusBody["capabilities"]
      mockReply({ http_status: 200, body: status })
      await expect(readStatus(), testCase.name).resolves.toEqual({ http_status: 200, body: status })
    }
  })

  it("keeps the daemon artifact-blocked control plane readable", async () => {
    const blocked = structuredClone(artifactBlockedStatusFixture) as StatusBody
    mockReply({ http_status: 200, body: blocked })
    await expect(readStatus()).resolves.toEqual({ http_status: 200, body: blocked })
    expect(daemonHealth({ http_status: 200, body: blocked })).toBe("degraded")
    expect(blocked.core).toEqual({ state: "blocked", reason: "artifact_unavailable" })

    const impossible = structuredClone(blocked)
    impossible.core.reason = "migration_rebuild"
    mockReply({ http_status: 200, body: impossible })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
  })

  it("accepts diagnostics v4 and rejects privacy or version drift", async () => {
    const body = diagnostics()
    mockReply({ http_status: 200, body })
    await expect(readDiagnostics()).resolves.toEqual({ http_status: 200, body })

    mockReply({ http_status: 200, body: { ...body, schema_version: "resume-ir.diagnostics.v3" } })
    await expect(readDiagnostics()).rejects.toMatchObject({ code: "daemon_contract" })
    mockReply({ http_status: 200, body: { ...body, contains_queries: true } })
    await expect(readDiagnostics()).rejects.toMatchObject({ code: "daemon_contract" })
  })

  it("rejects every non-200 error body on status and diagnostics", async () => {
    mockReply({ http_status: 400, body: badRequest })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
    await expect(readDiagnostics()).rejects.toMatchObject({ code: "daemon_contract" })
    const missing = structuredClone(badRequest) as unknown as { error: Record<string, unknown> }
    delete missing.error.capability
    mockReply({ http_status: 400, body: missing })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })

    const queryUnavailable = { schema_version: "resume-ir.error.v2", status: "error", error: { code: "QUERY_SERVICE_UNAVAILABLE", action: "repair_required", capability: null, reason: null } }
    mockReply({ http_status: 503, body: queryUnavailable })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
    await expect(readDiagnostics()).rejects.toMatchObject({ code: "daemon_contract" })
    mockReply({ http_status: 503, body: { ...queryUnavailable, error: { ...queryUnavailable.error, action: "retry" } } })
    await expect(readStatus()).rejects.toMatchObject({ code: "daemon_contract" })
  })
})

describe("lifecycle v2 bridge", () => {
  it("validates both lifecycle invokes before exposing them to the UI", async () => {
    const snapshot = runningLifecycle()
    mockReply(snapshot)
    await expect(getDaemonLifecycle()).resolves.toEqual(snapshot)
    await expect(retryDaemon()).resolves.toEqual(snapshot)
    expect(ipcCalls.map((call) => call.command)).toEqual(["get_daemon_lifecycle", "retry_daemon"])
  })

  it("turns legacy or illegal lifecycle payloads into bridge contract errors", async () => {
    mockReply({ ...runningLifecycle(), schema_version: "resume-ir.desktop-daemon-lifecycle.v1" })
    await expect(getDaemonLifecycle()).rejects.toMatchObject({ code: "daemon_contract" })
    mockReply({ ...runningLifecycle(), state: "blocked", transition_reason: "control_plane_ready" })
    await expect(retryDaemon()).rejects.toMatchObject({ code: "daemon_contract" })
  })
})

describe("operation projection and commands", () => {
  it("keeps errors distinct from empty/partial results and selection identity exact", () => {
    expect(searchDeadlineMs("keyword")).toBe(1500)
    expect(searchDeadlineMs("hybrid")).toBe(30000)
    expect(searchOutcome({ http_status: 400, body: badRequest })).toBe("error")
    expect(searchOutcome({ http_status: 503, body: { schema_version: "resume-ir.error.v2", request_id: "request-2", status: "error", error: { code: "OVERLOADED", action: "retry", retry_after_ms: 250, capability: null, reason: null } } })).toBe("overload")
    expect(searchOutcome({ http_status: 200, body: { schema_version: "resume-ir.search-response.v3", request_id: "request-3", status: "ok", visible_epoch: 1, query_mode: "keyword", partial: false, partial_reasons: [], latency_ms: 1, result_count: 0, results: [] } })).toBe("empty")
    const selection = { doc_id: "doc_00000000000000000000000000000000", version_id: "ver_00000000000000000000000000000000", visible_epoch: 7 }
    expect(sameSearchSelection(selection, { ...selection })).toBe(true)
    expect(sameSearchSelection(selection, { ...selection, visible_epoch: 8 })).toBe(false)
  })

  it("uses fixed IPC shapes and never sends a browser path for diagnostics", async () => {
    mockReply({ http_status: 200, body: readyStatus() })
    await readStatus()
    mockReply({ http_status: 200, body: diagnostics() })
    await readDiagnostics()
    mockReply(null)
    await expect(exportDiagnostics()).resolves.toBeNull()
    expect(ipcCalls).toEqual([
      { command: "daemon_request", payload: { request: { operation: "status" } } },
      { command: "daemon_request", payload: { request: { operation: "diagnostics" } } },
      { command: "export_diagnostics", payload: {} },
    ])
  })

  it("preserves exact search/detail/hydrate request identities without replay", async () => {
    mockReply({ http_status: 200, body: { status: "ok" } })
    const selection = { doc_id: "doc_00000000000000000000000000000000", version_id: "ver_00000000000000000000000000000000", visible_epoch: 7 }
    await searchResumes({ schema_version: "resume-ir.ipc-request.v3", request_id: "search-1", client_capability: "interactive_gui", deadline_ms: 1500, payload: { query: "synthetic", mode: "fulltext", top_k: 10, filters: {} } })
    await readDetail("detail-1", selection)
    await hydrateDetail("hydrate-1", selection, 1024)
    await requestSearchCancel("cancel-1", "cancel-token-1")
    expect(ipcCalls.map((call) => call.payload)).toEqual([
      { request: { operation: "search", body: { schema_version: "resume-ir.ipc-request.v3", request_id: "search-1", client_capability: "interactive_gui", deadline_ms: 1500, payload: { query: "synthetic", mode: "fulltext", top_k: 10, filters: {} } } } },
      { request: { operation: "detail", body: { schema_version: "resume-ir.detail-request.v3", request_id: "detail-1", selection } } },
      { request: { operation: "hydrate", body: { schema_version: "resume-ir.detail-hydrate-request.v3", request_id: "hydrate-1", selection, body_offset_bytes: 1024, body_limit_bytes: 32768 } } },
      { request: { operation: "cancel", body: { schema_version: "resume-ir.search-cancel-request.v1", request_id: "cancel-1", cancel_token: "cancel-token-1" } } },
    ])
  })

  it("projects managed-root v2 errors without a legacy reader", async () => {
    const accepted = { schema_version: "daemon.import.v1" as const, status: "accepted" as const, accepted_roots: 1, new_tasks: 1, scan_profile: "explicit" as const, scan_file_limit: null }
    expect(managedRootScanOutcome({ http_status: 202, body: accepted })).toBe("queued")
    const conflict: DaemonServiceErrorBody = { schema_version: "resume-ir.error.v2", status: "error", error: { code: "CONFLICT", action: "retry", capability: null, reason: null } }
    expect(managedRootScanOutcome({ http_status: 409, body: conflict })).toBe("active")
    const notFound: DaemonServiceErrorBody = { schema_version: "resume-ir.error.v2", status: "error", error: { code: "NOT_FOUND", action: "refresh_search", capability: null, reason: null } }
    expect(managedRootControlOutcome({ http_status: 404, body: notFound })).toBe("unmanaged")

    mockReply({ http_status: 200, body: { schema_version: "daemon.import_root_control.v1", status: "paused", changed: true, task_cancel_requested: true, catch_up_queued: false } })
    await controlManagedRoot("root-00000000000000000000000000000000", "pause")
    mockReply({ http_status: 202, body: accepted })
    await importSelectedRoot("root-00000000000000000000000000000000")
    expect(ipcCalls[0]).toEqual({ command: "daemon_request", payload: { request: { operation: "root_control", body: { root_handle: "root-00000000000000000000000000000000", action: "pause" } } } })
    expect(ipcCalls[1]).toEqual({ command: "import_selected_root", payload: { request: { root_handle: "root-00000000000000000000000000000000" } } })
  })
})
