import { useEffect, useRef, useState } from "react"

import {
  bridgeError,
  bridgeFailureKind,
  getDaemonLifecycle,
  listManagedRoots,
  readStatus,
  retryDaemon,
  type SourceRoot,
  type StatusBody,
} from "./daemon"
import { indexServicePresentation, lifecycleMessage, type RuntimeView } from "./daemon-health"
import {
  captureDaemonActionAuthority,
  captureLifecycleReadability,
  daemonActionAuthorityIsCurrent,
  initialDaemonActionAuthority,
  initialLifecycleReadability,
  invalidateLifecycleReadability,
  isDaemonLifecycleSnapshot,
  lifecycleReadabilityIsCurrent,
  lifecycleInvalidatesStatusAuthority,
  observeReadableLifecycle,
  reconcileResultFreshness,
  revokeDaemonActionAuthority,
  serviceStateFromStatus,
  startSerialLifecyclePolling,
  statusAuthorityIsCurrent,
  trustDaemonActionAuthority,
  type DaemonActionAuthorityToken,
  type DaemonLifecycleSnapshot,
  type DaemonService,
  type ResultFreshness,
} from "./runtime-state"

export type ImportState = "idle" | "selecting" | "selected" | "submitting" | "queued" | "pending" | "active" | "cancelled" | "unavailable" | "mismatch" | "overload" | "error"
export type RootControlState = "loading" | "unmanaged" | "active" | "paused" | "overload" | "error"
export type ManagedRootsReadFailure = "overload" | "error" | null
export type ManagedRootsRefreshOutcome = Exclude<ManagedRootsReadFailure, null> | "success"

export function managedRootsReadFailureAfterRefresh(
  outcome: ManagedRootsRefreshOutcome,
): ManagedRootsReadFailure {
  return outcome === "success" ? null : outcome
}

export function sourcePanelBanner(
  importState: ImportState,
  importMessage: string,
  managedRootsReadFailure: ManagedRootsReadFailure,
): { state: ImportState; message: string } {
  if (managedRootsReadFailure === "overload") {
    return { state: "overload", message: "授权目录读取入口繁忙，请稍后重试" }
  }
  if (managedRootsReadFailure === "error") {
    return { state: "error", message: "无法读取本地授权目录记录" }
  }
  return { state: importState, message: importMessage }
}

export function rootsAfterDeletionAccepted(
  roots: SourceRoot[],
  rootId: string,
): SourceRoot[] {
  return roots.map((root) => root.root_id === rootId ? { ...root, state: "deleting" } : root)
}

export function deletionReceiptUncertainPresentation(): {
  state: "submitting"
  message: string
} {
  return {
    state: "submitting",
    message: "目录删除接收状态未确认，正在重新读取授权目录",
  }
}

interface ResultSnapshot {
  generation: number
  visibleEpoch: number
}

interface DetailRuntimeObservers {
  observeAuthority(): void
  observeLifecycle(snapshot: DaemonLifecycleSnapshot): void
}

const NO_DETAIL_OBSERVERS: DetailRuntimeObservers = {
  observeAuthority: () => undefined,
  observeLifecycle: () => undefined,
}

const STARTING_LIFECYCLE: DaemonLifecycleSnapshot = {
  schema_version: "resume-ir.desktop-daemon-lifecycle.v2",
  state: "starting",
  transition_reason: "initial_start",
  generation: 0,
  automatic_restart_attempt: 0,
  automatic_restart_limit: 5,
  retry_after_ms: null,
  heartbeat_failures: 0,
  last_exit: null,
}

const PREVIEW_LIFECYCLE: DaemonLifecycleSnapshot = {
  ...STARTING_LIFECYCLE,
  state: "running",
  transition_reason: "control_plane_ready",
  generation: 1,
}

const PREVIEW_STATUS: StatusBody = {
  schema_version: "daemon.status.v6",
  status: "ok",
  process_state: "ready",
  core: { state: "ready", reason: null },
  writer: { state: "ready", reason: null, transition_phase: null, transition_id: null },
  optional_runtimes: {
    embedding: { state: "available", reason: null },
    ocr: { state: "available", reason: null },
    classifier: { state: "available", reason: null },
    pdfium: { state: "available", reason: null },
  },
  capabilities: {
    keyword_search: { state: "available", reason: null },
    detail: { state: "available", reason: null },
    semantic_search: { state: "available", reason: null },
    hybrid_search: { state: "available", reason: null },
    text_import: { state: "available", reason: null },
    pdf_import: { state: "available", reason: null },
    ocr_import: { state: "available", reason: null },
    index_publication: { state: "available", reason: null },
  },
  repair_progress: null,
  error: null,
  indexed_documents: 1284,
  searchable_documents: 1098,
  partial_documents: 84,
  visible_epoch: 1,
  failed_retryable: 2,
  failed_permanent: 1,
  recovery_queue_depth: 0,
  ocr_queue_depth: 102,
  ocr_jobs_queued: 102,
  ocr_page_budget_blocked: 0,
  ocr_remediation: "none",
  ocr_language_unavailable: 0,
  ocr_language_remediation: "none",
  embedding_queue_depth: 186,
  entity_mentions: 0,
  import_tasks_queued: 0,
  import_tasks_recoverable: 0,
  import_tasks_cancelled: 0,
  import_scan_scopes: 2,
  import_scan_errors: 1,
  query_latency: { sample_count: 8, p50_ms: 18, p95_ms: 42, p99_ms: 48, last_result_count: 5, raw_queries: "<redacted>" },
  latest_import_scan: {
    scan_profile: "explicit",
    files_discovered: 1284,
    ignored_entries: 0,
    scan_errors: 1,
    searchable_documents: 1098,
    ocr_required_documents: 102,
    ocr_jobs_queued: 102,
    failed_documents: 1,
    deleted_documents: 0,
    scan_budget_observed: null,
    scan_budget_limit: null,
    scan_budget_exhausted: false,
  },
  active_profile: "balanced",
  index_health: "ready",
  snapshot_present: true,
  ipc: { accepted: 8, completed: 8, client_disconnect: 0, request_failure: 0, response_failure: 0 },
}

const PREVIEW_SCAN = {
  scan_id: "imp_00000000000000000000000000000000",
  trigger: "periodic" as const,
  phase: "complete" as const,
  completeness: "complete" as const,
  counts: { discovered: 1284, searchable: 1098, non_resume: 83, needs_review: 0, ocr: 102, failed: 1, ignored: 0, processed: 1284, total: 1284, errors: 0 },
  rate_per_second: 12.4,
  eta_seconds: 0,
  started_at_seconds: 1,
  updated_at_seconds: 2,
  completed_at_seconds: 2,
}

const PREVIEW_MANAGED_ROOTS: SourceRoot[] = [
  { root_id: "root-00000000000000000000000000000000", display_label: "工程岗位简历", state: "active", watcher_state: "active", current_counts: { discovered: 1284, searchable: 1098, non_resume: 83, needs_review: 0, ocr: 102, failed: 1 }, last_scan: PREVIEW_SCAN },
  { root_id: "root-11111111111111111111111111111111", display_label: "外置盘历史简历", state: "offline", watcher_state: "unavailable", current_counts: { discovered: 318, searchable: 294, non_resume: 14, needs_review: 2, ocr: 8, failed: 0 }, last_scan: null },
]

const PREVIEW_ROOT_CONTROLS: Record<string, RootControlState> = {
  "root-00000000000000000000000000000000": "active",
  "root-11111111111111111111111111111111": "error",
}

const MAX_RETRY_AFTER_MS = 300_000

export function daemonRetryControl(snapshot: DaemonLifecycleSnapshot): { disabled: boolean; label: string } | null {
  if (snapshot.state === "blocked") return { disabled: false, label: "重新检测并启动" }
  if (snapshot.state !== "circuit_open") return null
  const retryAfterMs = Math.min(MAX_RETRY_AFTER_MS, Math.max(0, snapshot.retry_after_ms ?? 0))
  if (retryAfterMs === 0) return { disabled: false, label: "重新检测并启动" }
  return { disabled: true, label: `${Math.ceil(retryAfterMs / 1000)} 秒后可重试` }
}

export function useDaemonRuntime(input: {
  preview: boolean
  previewImport: boolean
  sourcePanelOpen: boolean
}) {
  const initialLifecycle = input.preview ? PREVIEW_LIFECYCLE : STARTING_LIFECYCLE
  const initialStatus = input.preview ? PREVIEW_STATUS : null
  const [lifecycle, setLifecycle] = useState<DaemonLifecycleSnapshot>(initialLifecycle)
  const [service, setService] = useState<DaemonService>(input.preview ? "ready" : "unknown")
  const [runtimeView, setRuntimeView] = useState<RuntimeView>(input.preview ? "trusted" : "service_unknown")
  const [resultFreshness, setResultFreshness] = useState<ResultFreshness>("current")
  const [connectionMessage, setConnectionMessage] = useState(input.preview ? "daemon 可用" : lifecycleMessage(STARTING_LIFECYCLE))
  const [status, setStatus] = useState<StatusBody | null>(initialStatus)
  const [statusGeneration, setStatusGeneration] = useState<number | null>(input.preview ? 1 : null)
  const [managedRoots, setManagedRoots] = useState<SourceRoot[]>(input.previewImport ? PREVIEW_MANAGED_ROOTS : [])
  const [rootControls, setRootControls] = useState<Record<string, RootControlState>>(input.previewImport ? PREVIEW_ROOT_CONTROLS : {})
  const [selectedRoot, setSelectedRoot] = useState<SourceRoot | null>(input.previewImport ? PREVIEW_MANAGED_ROOTS[0] : null)
  const [importState, setImportState] = useState<ImportState>(input.previewImport ? "selected" : "idle")
  const [importMessage, setImportMessage] = useState(input.previewImport ? "已恢复 2 个本地授权目录" : "选择一个本地目录后提交完整扫描")
  const [managedRootsReadFailure, setManagedRootsReadFailure] = useState<ManagedRootsReadFailure>(null)
  const lifecycleRef = useRef(initialLifecycle)
  const lifecycleReadabilityRef = useRef(initialLifecycleReadability(input.preview))
  const actionAuthorityRef = useRef(initialDaemonActionAuthority(input.preview ? 1 : null))
  const statusRef = useRef<StatusBody | null>(initialStatus)
  const statusGenerationRef = useRef<number | null>(input.preview ? 1 : null)
  const resultSnapshot = useRef<ResultSnapshot | null>(input.preview ? { generation: 1, visibleEpoch: 1 } : null)
  const managedRootsGeneration = useRef<number | null>(null)
  const retryInFlight = useRef(false)
  const detailObserversRef = useRef<DetailRuntimeObservers>(NO_DETAIL_OBSERVERS)
  const authoritativeStatus = status !== null && statusAuthorityIsCurrent(lifecycle, statusGeneration) ? status : null

  function bindDetailObservers(observers: DetailRuntimeObservers) {
    detailObserversRef.current = observers
  }

  function revokeActionAuthority() {
    actionAuthorityRef.current = revokeDaemonActionAuthority(actionAuthorityRef.current)
    detailObserversRef.current.observeAuthority()
  }

  function grantActionAuthority(generation: number): DaemonActionAuthorityToken | null {
    actionAuthorityRef.current = trustDaemonActionAuthority(actionAuthorityRef.current, generation)
    return captureDaemonActionAuthority(actionAuthorityRef.current, lifecycleRef.current)
  }

  function captureActionAuthority(): DaemonActionAuthorityToken | null {
    return captureDaemonActionAuthority(actionAuthorityRef.current, lifecycleRef.current)
  }

  function actionAuthorityIsCurrent(token: DaemonActionAuthorityToken): boolean {
    return daemonActionAuthorityIsCurrent(actionAuthorityRef.current, lifecycleRef.current, token)
  }

  function captureCapabilityAuthority(
    capability: keyof StatusBody["capabilities"],
    allowDegraded = false,
  ): DaemonActionAuthorityToken | null {
    const authority = captureActionAuthority()
    const currentStatus = statusRef.current
    if (!authority || !currentStatus || statusGenerationRef.current !== authority.generation) return null
    const state = currentStatus.capabilities[capability].state
    return state === "available" || (allowDegraded && state === "degraded") ? authority : null
  }

  function capabilityAuthorityIsCurrent(
    token: DaemonActionAuthorityToken,
    capability: keyof StatusBody["capabilities"],
    allowDegraded = false,
  ): boolean {
    if (!actionAuthorityIsCurrent(token) || statusGenerationRef.current !== token.generation) return false
    const state = statusRef.current?.capabilities[capability].state
    return state === "available" || (allowDegraded && state === "degraded")
  }

  function capabilityAuthorized(capability: keyof StatusBody["capabilities"], allowDegraded = false): boolean {
    return captureCapabilityAuthority(capability, allowDegraded) !== null
  }

  async function refreshStatus(): Promise<DaemonActionAuthorityToken | null> {
    if (input.preview) return captureActionAuthority()
    const requestedReadability = captureLifecycleReadability(lifecycleReadabilityRef.current)
    const requestedLifecycle = lifecycleRef.current
    if (requestedReadability === null || requestedLifecycle.state !== "running") {
      revokeActionAuthority()
      return null
    }
    const requestedGeneration = requestedLifecycle.generation
    try {
      const reply = await readStatus()
      const currentLifecycle = lifecycleRef.current
      if (
        !lifecycleReadabilityIsCurrent(lifecycleReadabilityRef.current, requestedReadability)
        || currentLifecycle.state !== "running"
        || currentLifecycle.generation !== requestedGeneration
      ) return null
      const body = reply.body.schema_version === "daemon.status.v6" ? reply.body : null
      if (reply.http_status !== 200 || body === null) throw new Error("daemon status contract mismatch")
      statusGenerationRef.current = requestedGeneration
      statusRef.current = body
      setStatusGeneration(requestedGeneration)
      setStatus(body)
      setRuntimeView("trusted")
      const nextService = serviceStateFromStatus({ httpStatus: reply.http_status, status: body.core.state })
      setService(nextService)
      const result = resultSnapshot.current
      setResultFreshness((current) => reconcileResultFreshness({
        current,
        hasResults: result !== null,
        resultGeneration: result?.generation ?? null,
        resultVisibleEpoch: result?.visibleEpoch ?? null,
        lifecycle: lifecycleRef.current,
        serviceVisibleEpoch: body.visible_epoch,
      }))
      setConnectionMessage(indexServicePresentation(nextService, body.core.reason).message)
      if (body.core.state !== "ready") {
        revokeActionAuthority()
        return null
      }
      return grantActionAuthority(requestedGeneration)
    } catch (error) {
      const currentLifecycle = lifecycleRef.current
      if (
        !lifecycleReadabilityIsCurrent(lifecycleReadabilityRef.current, requestedReadability)
        || currentLifecycle.state !== "running"
        || currentLifecycle.generation !== requestedGeneration
      ) return null
      clearStatusAuthority("service_unknown")
      setConnectionMessage("daemon 生命周期可读，但服务状态未知；操作权限已撤销")
      revokeActionAuthority()
      return null
    }
  }

  function clearStatusAuthority(nextRuntimeView: RuntimeView) {
    statusRef.current = null
    statusGenerationRef.current = null
    setStatusGeneration(null)
    setStatus(null)
    setService("unknown")
    setRuntimeView(nextRuntimeView)
  }

  function applyLifecycleSnapshot(snapshot: DaemonLifecycleSnapshot) {
    const previous = lifecycleRef.current
    const revokeStatus = lifecycleInvalidatesStatusAuthority(previous, snapshot, statusGenerationRef.current)
    lifecycleReadabilityRef.current = observeReadableLifecycle(lifecycleReadabilityRef.current)
    lifecycleRef.current = snapshot
    setLifecycle(snapshot)
    const result = resultSnapshot.current
    setResultFreshness((current) => reconcileResultFreshness({
      current,
      hasResults: result !== null,
      resultGeneration: result?.generation ?? null,
      resultVisibleEpoch: result?.visibleEpoch ?? null,
      lifecycle: snapshot,
      serviceVisibleEpoch: null,
    }))
    detailObserversRef.current.observeLifecycle(snapshot)
    if (revokeStatus) {
      clearStatusAuthority(snapshot.state === "running" ? "service_unknown" : "trusted")
      setConnectionMessage(snapshot.state === "running" ? "daemon 已换代，正在读取新一代服务状态" : lifecycleMessage(snapshot))
      revokeActionAuthority()
    } else {
      setRuntimeView("trusted")
    }
  }

  async function retryLifecycle() {
    if (input.preview || retryInFlight.current) return
    if (daemonRetryControl(lifecycleRef.current)?.disabled) return
    retryInFlight.current = true
    setConnectionMessage("正在请求 daemon 监督器重试")
    try {
      const snapshot = await retryDaemon()
      if (!isDaemonLifecycleSnapshot(snapshot)) throw new Error("lifecycle contract mismatch")
      applyLifecycleSnapshot(snapshot)
    } catch (error) {
      lifecycleReadabilityRef.current = invalidateLifecycleReadability(lifecycleReadabilityRef.current)
      clearStatusAuthority("bridge_error")
      setConnectionMessage(bridgeFailureKind(error) === "overload"
        ? "生命周期重试入口繁忙；操作权限已撤销"
        : bridgeError(error).message)
      revokeActionAuthority()
    } finally {
      retryInFlight.current = false
    }
  }

  async function refreshManagedRoots(announce = false) {
    if (input.preview) return
    try {
      const reply = await listManagedRoots()
      const response = reply.body
      if (reply.http_status !== 200 || response.schema_version !== "resume-ir.source-roots.v2" || response.limit !== 16 || response.roots.length > response.limit) {
        throw new Error("managed root contract mismatch")
      }
      setManagedRootsReadFailure(managedRootsReadFailureAfterRefresh("success"))
      setManagedRoots(response.roots)
      setSelectedRoot((current) => {
        const restored = current && response.roots.find((root) => root.root_id === current.root_id)
        return restored ?? response.roots.find((root) => root.state === "active") ?? response.roots[0] ?? null
      })
      if (announce && response.roots.length > 0) {
        const available = response.roots.filter((root) => root.state === "active").length
        const deleting = response.roots.filter((root) => root.state === "deleting").length
        setImportState(available > 0 ? "selected" : deleting > 0 ? "active" : "unavailable")
        setImportMessage(
          available > 0
            ? `已恢复 ${response.roots.length} 个本地授权目录`
            : deleting > 0
              ? `正在继续清理 ${deleting} 个目录的本地派生数据`
              : "授权目录当前均不可读取",
        )
      } else if (announce) {
        setImportState("idle")
        setImportMessage("选择一个本地目录后开始扫描")
      }
      setRootControls(Object.fromEntries(response.roots.map((root) => [
        root.root_id,
        root.state === "deleting"
          ? "loading"
          : root.state === "offline"
          ? "error"
          : root.watcher_state === "paused"
            ? "paused"
            : root.watcher_state === "active"
              ? "active"
              : "error",
      ])))
    } catch (error) {
      const overload = bridgeFailureKind(error) === "overload"
      setManagedRootsReadFailure(managedRootsReadFailureAfterRefresh(overload ? "overload" : "error"))
    }
  }

  useEffect(() => {
    if (input.preview) return
    return startSerialLifecyclePolling({
      readSnapshot: getDaemonLifecycle,
      onSnapshot: async (snapshot) => {
        if (!isDaemonLifecycleSnapshot(snapshot)) throw new Error("lifecycle contract mismatch")
        applyLifecycleSnapshot(snapshot)
        if (snapshot.state === "running") {
          const authority = await refreshStatus()
          if (authority && actionAuthorityIsCurrent(authority)) {
            const generationChanged = managedRootsGeneration.current !== snapshot.generation
            if (generationChanged || input.sourcePanelOpen) {
              managedRootsGeneration.current = snapshot.generation
              await refreshManagedRoots(generationChanged)
            }
          }
        }
      },
      onError: (error) => {
        lifecycleReadabilityRef.current = invalidateLifecycleReadability(lifecycleReadabilityRef.current)
        clearStatusAuthority("bridge_error")
        setConnectionMessage(`生命周期不可读：${bridgeError(error).message}；操作权限已撤销`)
        revokeActionAuthority()
      },
      clock: {
        setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
        clearTimeout: (timer) => window.clearTimeout(timer),
      },
      focusEvents: {
        addFocusListener: (listener) => window.addEventListener("focus", listener),
        removeFocusListener: (listener) => window.removeEventListener("focus", listener),
      },
    })
  }, [input.preview, input.sourcePanelOpen])

  return {
    lifecycle,
    lifecycleRef,
    actionAuthorityRef,
    service,
    setService,
    runtimeView,
    resultFreshness,
    setResultFreshness,
    connectionMessage,
    authoritativeStatus,
    resultSnapshot,
    managedRoots,
    setManagedRoots,
    rootControls,
    setRootControls,
    selectedRoot,
    setSelectedRoot,
    importState,
    setImportState,
    importMessage,
    setImportMessage,
    managedRootsReadFailure,
    bindDetailObservers,
    captureActionAuthority,
    actionAuthorityIsCurrent,
    captureCapabilityAuthority,
    capabilityAuthorityIsCurrent,
    capabilityAuthorized,
    refreshStatus,
    retryLifecycle,
    refreshManagedRoots,
  }
}
