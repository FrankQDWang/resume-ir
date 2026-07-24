import { useEffect, useRef, useState } from "react"

import {
  bridgeError,
  bridgeFailureKind,
  controlManagedRoot,
  getDaemonLifecycle,
  listManagedRoots,
  managedRootControlOutcome,
  readStatus,
  retryDaemon,
  type ManagedRoot,
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

export type ImportState = "idle" | "selecting" | "selected" | "submitting" | "reauthorizing" | "queued" | "pending" | "active" | "cancelled" | "unavailable" | "mismatch" | "overload" | "error"
export type RootControlState = "loading" | "unmanaged" | "active" | "paused" | "overload" | "error"

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
  schema_version: "daemon.status.v3",
  status: "ok",
  process_state: "ready",
  core: { state: "ready", reason: null },
  optional_runtimes: {
    embedding: { state: "available", reason: null },
    ocr: { state: "available", reason: null },
    classifier: { state: "available", reason: null },
  },
  capabilities: {
    keyword_search: { state: "available", reason: null },
    detail: { state: "available", reason: null },
    semantic_search: { state: "available", reason: null },
    hybrid_search: { state: "available", reason: null },
    text_import: { state: "available", reason: null },
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

const PREVIEW_MANAGED_ROOTS: ManagedRoot[] = [
  { root_handle: "root-00000000000000000000000000000000", display_label: "工程岗位简历", availability: "available" },
  { root_handle: "root-11111111111111111111111111111111", display_label: "外置盘历史简历", availability: "unavailable" },
]

const PREVIEW_ROOT_CONTROLS: Record<string, RootControlState> = {
  "root-00000000000000000000000000000000": "active",
  "root-11111111111111111111111111111111": "paused",
}

const MAX_RETRY_AFTER_MS = 300_000

export function daemonRetryControl(snapshot: DaemonLifecycleSnapshot): { disabled: boolean; label: string } | null {
  if (snapshot.state === "blocked") return { disabled: false, label: "重新检测并启动" }
  if (snapshot.state !== "circuit_open") return null
  const retryAfterMs = Math.min(MAX_RETRY_AFTER_MS, Math.max(0, snapshot.retry_after_ms ?? 0))
  if (retryAfterMs === 0) return { disabled: false, label: "重新检测并启动" }
  return { disabled: true, label: `${Math.ceil(retryAfterMs / 1000)} 秒后可重试` }
}

export function useDaemonRuntime(input: { preview: boolean; previewImport: boolean }) {
  const initialLifecycle = input.preview ? PREVIEW_LIFECYCLE : STARTING_LIFECYCLE
  const initialStatus = input.preview ? PREVIEW_STATUS : null
  const [lifecycle, setLifecycle] = useState<DaemonLifecycleSnapshot>(initialLifecycle)
  const [service, setService] = useState<DaemonService>(input.preview ? "ready" : "unknown")
  const [runtimeView, setRuntimeView] = useState<RuntimeView>(input.preview ? "trusted" : "service_unknown")
  const [resultFreshness, setResultFreshness] = useState<ResultFreshness>("current")
  const [connectionMessage, setConnectionMessage] = useState(input.preview ? "daemon 可用" : lifecycleMessage(STARTING_LIFECYCLE))
  const [status, setStatus] = useState<StatusBody | null>(initialStatus)
  const [statusGeneration, setStatusGeneration] = useState<number | null>(input.preview ? 1 : null)
  const [managedRoots, setManagedRoots] = useState<ManagedRoot[]>(input.previewImport ? PREVIEW_MANAGED_ROOTS : [])
  const [rootControls, setRootControls] = useState<Record<string, RootControlState>>(input.previewImport ? PREVIEW_ROOT_CONTROLS : {})
  const [selectedRoot, setSelectedRoot] = useState<ManagedRoot | null>(input.previewImport ? PREVIEW_MANAGED_ROOTS[0] : null)
  const [importState, setImportState] = useState<ImportState>(input.previewImport ? "selected" : "idle")
  const [importMessage, setImportMessage] = useState(input.previewImport ? "已恢复 2 个本地授权目录" : "选择一个本地目录后提交完整扫描")
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
      const body = reply.body.schema_version === "daemon.status.v3" ? reply.body : null
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

  async function refreshManagedRoots() {
    if (input.preview) return
    try {
      const response = await listManagedRoots()
      if (response.schema_version !== "resume-ir.desktop-managed-roots.v1" || response.limit !== 16 || response.roots.length > response.limit) {
        throw new Error("managed root contract mismatch")
      }
      setManagedRoots(response.roots)
      setSelectedRoot((current) => {
        const restored = current && response.roots.find((root) => root.root_handle === current.root_handle)
        return restored ?? response.roots.find((root) => root.availability === "available") ?? response.roots[0] ?? null
      })
      if (response.roots.length > 0) {
        const available = response.roots.filter((root) => root.availability === "available").length
        setImportState(available > 0 ? "selected" : "unavailable")
        setImportMessage(available > 0 ? `已恢复 ${response.roots.length} 个本地授权目录` : "授权目录当前均不可读取")
      }
      await inspectRootControls(response.roots)
    } catch (error) {
      const overload = bridgeFailureKind(error) === "overload"
      setImportState(overload ? "overload" : "error")
      setImportMessage(overload ? "授权目录读取入口繁忙，请稍后重试" : "无法读取本地授权目录记录")
    }
  }

  async function inspectRootControls(roots: ManagedRoot[]) {
    const next: Record<string, RootControlState> = {}
    for (const root of roots) {
      const authority = captureActionAuthority()
      if (!authority) return
      next[root.root_handle] = "loading"
      setRootControls({ ...next })
      try {
        const reply = await controlManagedRoot(root.root_handle, "inspect")
        if (!actionAuthorityIsCurrent(authority)) return
        next[root.root_handle] = managedRootControlOutcome(reply)
      } catch (error) {
        if (!actionAuthorityIsCurrent(authority)) return
        next[root.root_handle] = bridgeFailureKind(error) === "overload" ? "overload" : "error"
      }
    }
    setRootControls(next)
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
          if (authority && actionAuthorityIsCurrent(authority) && managedRootsGeneration.current !== snapshot.generation) {
            managedRootsGeneration.current = snapshot.generation
            await refreshManagedRoots()
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
  }, [input.preview])

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
