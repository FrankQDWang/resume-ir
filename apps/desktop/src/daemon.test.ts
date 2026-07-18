import { clearMocks, mockIPC } from "@tauri-apps/api/mocks"
import { beforeEach, describe, expect, it } from "vitest"

import {
  bridgeError,
  bridgeFailureKind,
  controlManagedRoot,
  daemonHealth,
  exportDiagnostics,
  getDaemonLifecycle,
  hydrateDetail,
  importSelectedRoot,
  listManagedRoots,
  managedRootControlOutcome,
  managedRootRecoveryFailure,
  managedRootScanOutcome,
  readDetail,
  readDiagnostics,
  readStatus,
  reauthorizeManagedRoot,
  requestSearchCancel,
  retryDaemon,
  rescanManagedRoot,
  searchDeadlineMs,
  searchOutcome,
  searchResumes,
  sameSearchSelection,
  selectImportRoot,
} from "./daemon"

interface IpcCall {
  command: string
  payload: unknown
}

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

describe("desktop bridge errors", () => {
  it("keeps bounded native bridge errors", () => {
    expect(bridgeError({ code: "daemon_unavailable", message: "本地 daemon 未运行或尚未就绪" })).toEqual({
      code: "daemon_unavailable",
      message: "本地 daemon 未运行或尚未就绪",
    })
  })

  it("does not expose arbitrary thrown values", () => {
    const error = bridgeError(new Error("/private/local/path/ipc.auth"))
    expect(error).toEqual({ code: "bridge_error", message: "桌面桥接请求失败" })
    expect(error.message).not.toContain("/private/local/path")
  })

  it("keeps bridge admission overload distinct from unavailable and generic errors", () => {
    expect(bridgeFailureKind({ code: "bridge_overloaded", message: "桌面请求繁忙，请稍后重试" })).toBe("overload")
    expect(bridgeFailureKind({ code: "daemon_unavailable", message: "本地 daemon 暂时不可用" })).toBe("unavailable")
    expect(bridgeFailureKind({ code: "daemon_generation_changed", message: "daemon 已换代" })).toBe("unavailable")
    expect(bridgeFailureKind({ code: "REPAIRING", message: "索引正在修复" })).toBe("unavailable")
    expect(bridgeFailureKind({ code: "METADATA_UNAVAILABLE", message: "元数据不可用" })).toBe("unavailable")
    expect(bridgeFailureKind({ code: "QUERY_SERVICE_UNAVAILABLE", message: "查询不可用" })).toBe("unavailable")
    expect(bridgeFailureKind({ code: "STALE_SELECTION", message: "需要刷新搜索" })).toBe("stale_selection")
    expect(bridgeFailureKind({ code: "NOT_FOUND", message: "目标版本不存在" })).toBe("selection_missing")
    expect(bridgeFailureKind(new Error("synthetic failure"))).toBe("error")
  })
})

describe("search product states", () => {
  it("keeps keyword deadlines fast while bounding local semantic startup", () => {
    expect(searchDeadlineMs("keyword")).toBe(1500)
    expect(searchDeadlineMs("field")).toBe(1500)
    expect(searchDeadlineMs("hybrid")).toBe(30000)
    expect(searchDeadlineMs("semantic")).toBe(30000)
  })

  it("keeps daemon errors distinct from legitimate empty results", () => {
    expect(searchOutcome({ http_status: 400, body: { schema_version: "resume-ir.error.v1", request_id: "request-1", status: "error", error: { code: "BAD_REQUEST", action: "correct_request" } } })).toBe("error")
    expect(searchOutcome({ http_status: 503, body: { schema_version: "resume-ir.error.v1", status: "error", error: { code: "REPAIRING", action: "wait_for_repair" } } })).toBe("error")
    expect(searchOutcome({ http_status: 503, body: { schema_version: "resume-ir.error.v1", request_id: "request-2", status: "error", error: { code: "OVERLOADED", action: "retry", retry_after_ms: 250 } } })).toBe("overload")
    expect(searchOutcome({ http_status: 200, body: { schema_version: "resume-ir.search-response.v3", request_id: "request-3", status: "ok", visible_epoch: 1, query_mode: "keyword", partial: false, partial_reasons: [], latency_ms: 1, result_count: 0, results: [] } })).toBe("empty")
  })

  it("treats a search selection as one indivisible identity", () => {
    const selection = { doc_id: "doc_00000000000000000000000000000000", version_id: "ver_00000000000000000000000000000000", visible_epoch: 7 }
    expect(sameSearchSelection(selection, { ...selection })).toBe(true)
    expect(sameSearchSelection(selection, { ...selection, visible_epoch: 8 })).toBe(false)
  })
})

describe("daemon health product states", () => {
  const status = (serviceState: "ready" | "repairing" | "degraded") => ({
    schema_version: "daemon.status.v2" as const,
    status: serviceState === "ready" ? "ok" as const : serviceState === "repairing" ? "repairing" as const : "degraded" as const,
    process_state: "ready" as const,
    service_state: serviceState,
    services: {
      metadata: "ready" as const,
      query: serviceState === "ready" ? "ready" as const : serviceState === "repairing" ? "repairing" as const : "unavailable" as const,
    },
    error: serviceState === "ready" ? null : serviceState === "repairing"
      ? { code: "REPAIRING" as const, action: "wait_for_repair" as const }
      : { code: "QUERY_SERVICE_UNAVAILABLE" as const, action: "retry" as const },
    indexed_documents: 0,
    searchable_documents: 0,
    partial_documents: 0,
    visible_epoch: 7,
    failed_retryable: 0,
    failed_permanent: 0,
    recovery_queue_depth: 0,
    ocr_queue_depth: 0,
    embedding_queue_depth: 0,
    entity_mentions: 0,
    import_tasks_queued: 0,
    index_health: serviceState === "ready" ? "ready" as const : "stale" as const,
    latest_import_scan: null,
    ipc: { accepted: 1, completed: 1, client_disconnect: 0, request_failure: 0, response_failure: 0 },
  })

  it("only treats a ready service contract as healthy", () => {
    expect(daemonHealth({ http_status: 200, body: status("ready") })).toBe("ok")
    expect(daemonHealth({ http_status: 200, body: status("repairing") })).toBe("degraded")
    expect(daemonHealth({ http_status: 200, body: status("degraded") })).toBe("degraded")
  })

  it("keeps an unhealthy HTTP reply degraded even if its payload says ready", () => {
    expect(daemonHealth({ http_status: 503, body: status("ready") })).toBe("degraded")
  })
})

describe("native import commands", () => {
  it("reads and retries the native-owned supervisor without asking the WebView to start a process", async () => {
    const snapshot = {
      schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
      state: "recovering",
      generation: 2,
      restart_attempt: 1,
      restart_budget: 5,
      retry_delay_ms: 250,
      consecutive_heartbeat_failures: 0,
      blocked_reason: null,
      last_exit: "child_exited",
    }
    mockReply(snapshot)

    await expect(getDaemonLifecycle()).resolves.toEqual(snapshot)
    await expect(retryDaemon()).resolves.toEqual(snapshot)
    expect(ipcCalls).toEqual([
      { command: "get_daemon_lifecycle", payload: {} },
      { command: "retry_daemon", payload: {} },
    ])
  })

  it("opens the native picker without passing browser filesystem state", async () => {
    mockReply(null)

    await expect(selectImportRoot()).resolves.toBeNull()
    expect(ipcCalls).toEqual([{ command: "select_import_root", payload: {} }])
  })

  it("submits only the opaque root handle from the WebView", async () => {
    mockReply({ http_status: 202, body: { status: "accepted" } })

    await importSelectedRoot("root-synthetic-handle")
    expect(ipcCalls).toEqual([{
      command: "import_selected_root",
      payload: { request: { root_handle: "root-synthetic-handle" } },
    }])
  })

  it("lists only bounded opaque managed-root state", async () => {
    mockReply({
      schema_version: "resume-ir.desktop-managed-roots.v1",
      limit: 16,
      roots: [{
        root_handle: "root-00000000000000000000000000000000",
        display_label: "synthetic-root",
        availability: "available",
      }],
    })

    await expect(listManagedRoots()).resolves.toEqual({
      schema_version: "resume-ir.desktop-managed-roots.v1",
      limit: 16,
      roots: [{
        root_handle: "root-00000000000000000000000000000000",
        display_label: "synthetic-root",
        availability: "available",
      }],
    })
    expect(ipcCalls).toEqual([{ command: "list_managed_roots", payload: {} }])
  })

  it("rescans one managed root through the existing opaque-handle command", async () => {
    mockReply({
      http_status: 202,
      body: {
        schema_version: "daemon.import.v1",
        status: "accepted",
        accepted_roots: 1,
        new_tasks: 1,
        scan_profile: "explicit",
        scan_file_limit: null,
      },
    })

    await rescanManagedRoot("root-00000000000000000000000000000000")
    expect(ipcCalls).toEqual([{
      command: "import_selected_root",
      payload: {
        request: { root_handle: "root-00000000000000000000000000000000" },
      },
    }])
  })

  it("keeps queued, pending, active, and failed managed-root scans distinct", () => {
    const accepted = {
      schema_version: "daemon.import.v1" as const,
      status: "accepted" as const,
      accepted_roots: 1,
      scan_profile: "explicit" as const,
      scan_file_limit: null,
    }
    expect(managedRootScanOutcome({ http_status: 202, body: { ...accepted, new_tasks: 1 } })).toBe("queued")
    expect(managedRootScanOutcome({ http_status: 202, body: { ...accepted, new_tasks: 0 } })).toBe("pending")
    expect(managedRootScanOutcome({ http_status: 409, body: { schema_version: "daemon.error.v1", status: "conflict" } })).toBe("active")
    expect(managedRootScanOutcome({ http_status: 500, body: { schema_version: "daemon.error.v1", status: "internal" } })).toBe("error")
  })

  it("controls one managed root through the existing command with only an opaque handle and action", async () => {
    mockReply({
      http_status: 200,
      body: {
        schema_version: "daemon.import_root_control.v1",
        status: "paused",
        changed: true,
        task_cancel_requested: true,
        catch_up_queued: false,
      },
    })

    await controlManagedRoot("root-00000000000000000000000000000000", "pause")
    expect(ipcCalls).toEqual([{
      command: "daemon_request",
      payload: {
        request: {
          operation: "root_control",
          body: {
            root_handle: "root-00000000000000000000000000000000",
            action: "pause",
          },
        },
      },
    }])
  })

  it("keeps managed-root control states distinct and recoverable", () => {
    const body = {
      schema_version: "daemon.import_root_control.v1" as const,
      changed: false,
      task_cancel_requested: false,
      catch_up_queued: false,
    }
    expect(managedRootControlOutcome({ http_status: 200, body: { ...body, status: "active" } })).toBe("active")
    expect(managedRootControlOutcome({ http_status: 200, body: { ...body, status: "paused" } })).toBe("paused")
    expect(managedRootControlOutcome({ http_status: 404, body: { schema_version: "daemon.error.v1", status: "not_found" } })).toBe("unmanaged")
    expect(managedRootControlOutcome({ http_status: 503, body: { schema_version: "daemon.error.v1", status: "internal" } })).toBe("error")
  })

  it("reauthorizes only one existing opaque managed-root handle", async () => {
    mockReply(null)

    await expect(reauthorizeManagedRoot("root-11111111111111111111111111111111")).resolves.toBeNull()
    expect(ipcCalls).toEqual([{
      command: "reauthorize_managed_root",
      payload: {
        request: { root_handle: "root-11111111111111111111111111111111" },
      },
    }])
  })

  it("keeps managed-root recovery failures distinct", () => {
    expect(managedRootRecoveryFailure({ code: "bridge_overloaded", message: "bounded" })).toBe("overload")
    expect(managedRootRecoveryFailure({ code: "managed_root_mismatch", message: "bounded" })).toBe("mismatch")
    expect(managedRootRecoveryFailure({ code: "import_root_unreadable", message: "bounded" })).toBe("unavailable")
    expect(managedRootRecoveryFailure({ code: "bridge_error", message: "bounded" })).toBe("error")
  })

  it("opens native diagnostics export without a browser path argument", async () => {
    mockReply(null)

    await expect(exportDiagnostics()).resolves.toBeNull()
    expect(ipcCalls).toEqual([{ command: "export_diagnostics", payload: {} }])
  })
})

describe("typed daemon operation commands", () => {
  it("uses one fixed request shape per operation without a generic unknown caller", async () => {
    mockReply({ http_status: 200, body: { status: "ok" } })
    const selection = {
      doc_id: "doc_00000000000000000000000000000000",
      version_id: "ver_00000000000000000000000000000000",
      visible_epoch: 7,
    }

    await readStatus()
    await readDiagnostics()
    await searchResumes({
      schema_version: "resume-ir.ipc-request.v3",
      request_id: "gui-search-synthetic",
      client_capability: "interactive_gui",
      deadline_ms: 1500,
      cancel_token: "gui-cancel-synthetic",
      payload: { query: "synthetic query", mode: "fulltext", top_k: 50, filters: {} },
    })
    await readDetail("gui-detail-synthetic", selection)
    await requestSearchCancel("gui-cancel-command-synthetic", "gui-cancel-synthetic")

    expect(ipcCalls).toEqual([
      { command: "daemon_request", payload: { request: { operation: "status" } } },
      { command: "daemon_request", payload: { request: { operation: "diagnostics" } } },
      {
        command: "daemon_request",
        payload: {
          request: {
            operation: "search",
            body: {
              schema_version: "resume-ir.ipc-request.v3",
              request_id: "gui-search-synthetic",
              client_capability: "interactive_gui",
              deadline_ms: 1500,
              cancel_token: "gui-cancel-synthetic",
              payload: { query: "synthetic query", mode: "fulltext", top_k: 50, filters: {} },
            },
          },
        },
      },
      {
        command: "daemon_request",
        payload: {
          request: {
            operation: "detail",
            body: {
              schema_version: "resume-ir.detail-request.v3",
              request_id: "gui-detail-synthetic",
              selection,
            },
          },
        },
      },
      {
        command: "daemon_request",
        payload: {
          request: {
            operation: "cancel",
            body: {
              schema_version: "resume-ir.search-cancel-request.v1",
              request_id: "gui-cancel-command-synthetic",
              cancel_token: "gui-cancel-synthetic",
            },
          },
        },
      },
    ])
  })
})

describe("local detail hydration", () => {
  it("requests one bounded authenticated body page without exposing a path", async () => {
    mockReply({ http_status: 200, body: { status: "ok" } })
    const selection = {
      doc_id: "doc_00000000000000000000000000000000",
      version_id: "ver_00000000000000000000000000000000",
      visible_epoch: 7,
    }

    await hydrateDetail("gui-hydrate-synthetic", selection, 32768)
    expect(ipcCalls).toEqual([
      {
        command: "daemon_request",
        payload: {
          request: {
            operation: "hydrate",
            body: {
              schema_version: "resume-ir.detail-hydrate-request.v3",
              request_id: "gui-hydrate-synthetic",
              selection,
              body_offset_bytes: 32768,
              body_limit_bytes: 32768,
            },
          },
        },
      },
    ])
  })
})
