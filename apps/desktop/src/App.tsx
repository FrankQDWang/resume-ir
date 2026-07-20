import { type FormEvent, type ReactNode, useEffect, useMemo, useRef, useState } from "react"
import {
  AlertTriangle,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Download,
  FileText,
  FolderOpen,
  FolderTree,
  HardDriveDownload,
  LoaderCircle,
  MapPin,
  Pause,
  Play,
  RefreshCw,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Upload,
  X,
} from "lucide-react"
import {
  bridgeError,
  bridgeFailureKind,
  controlManagedRoot,
  exportDiagnostics,
  getDaemonLifecycle,
  importSelectedRoot,
  listManagedRoots,
  managedRootControlOutcome,
  managedRootRecoveryFailure,
  managedRootScanOutcome,
  readDiagnostics,
  readStatus,
  reauthorizeManagedRoot,
  requestSearchCancel,
  retryDaemon,
  rescanManagedRoot,
  searchDeadlineMs,
  searchOutcome,
  searchResumes,
  selectImportRoot,
  type DiagnosticsBody,
  type ManagedRoot,
  type ManagedRootControlAction,
  type SearchHit,
  type StatusBody,
} from "./daemon"
import { MAX_DETAIL_PAGES, useDetailSession } from "./detail-session"
import {
  blockedReasonMessage,
  detailReadMayContinue,
  isDaemonLifecycleSnapshot,
  reconcileResultFreshness,
  serviceStateFromStatus,
  startSerialLifecyclePolling,
  type DaemonLifecycleSnapshot,
  type DaemonService,
  type ResultFreshness,
} from "./runtime-state"

type ViewState = "idle" | "loading" | "complete" | "partial" | "empty" | "overload" | "cancelled" | "error"
type ImportState = "idle" | "selecting" | "selected" | "submitting" | "reauthorizing" | "queued" | "pending" | "active" | "cancelled" | "unavailable" | "mismatch" | "overload" | "error"
type DiagnosticsState = "idle" | "loading" | "ready" | "exporting" | "saved" | "cancelled" | "blocked" | "overload" | "error"
type Mode = "keyword" | "field" | "hybrid" | "semantic"
type Degree = "" | "associate" | "bachelor" | "master" | "doctorate"
type Overlay = "import" | "diagnostics" | null
type RootControlState = "loading" | "unmanaged" | "active" | "paused" | "overload" | "error"

const MODE_OPTIONS: Array<{ value: Mode; label: string }> = [
  { value: "keyword", label: "关键词" },
  { value: "field", label: "字段过滤" },
  { value: "hybrid", label: "混合" },
  { value: "semantic", label: "语义" },
]
const RESULT_PAGE_SIZE = 4
const PREVIEW_RESULTS: SearchHit[] = [
  { rank: 1, selection: { doc_id: "doc_preview_01", version_id: "ver_preview_01", visible_epoch: 1 }, file_name: "张伟_高级后端工程师.pdf", snippet: "负责核心支付清结算系统，基于 Java 与 Kafka 构建高吞吐消息管道，QPS 提升 3 倍。" },
  { rank: 2, selection: { doc_id: "doc_preview_02", version_id: "ver_preview_02", visible_epoch: 1 }, file_name: "李娜_支付平台架构师.docx", snippet: "主导支付网关重构，使用 Java 与 Kafka 实现异步对账与削峰。" },
  { rank: 3, selection: { doc_id: "doc_preview_03", version_id: "ver_preview_03", visible_epoch: 1 }, file_name: "王强_物流研发工程师.pdf", snippet: "使用 Java 与 Kafka 搭建物流轨迹实时计算管道。" },
  { rank: 4, selection: { doc_id: "doc_preview_04", version_id: "ver_preview_04", visible_epoch: 1 }, file_name: "候选人_扫描简历.pdf", snippet: "Java 高级开发，熟悉 Kafka、分布式系统与高并发服务治理。" },
  { rank: 5, selection: { doc_id: "doc_preview_05", version_id: "ver_preview_05", visible_epoch: 1 }, file_name: "陈晨_服务端开发.pdf", snippet: "服务端研发，参与交易系统与事件驱动架构建设。" },
]
const PREVIEW_MANAGED_ROOTS: ManagedRoot[] = [
  { root_handle: "root-00000000000000000000000000000000", display_label: "工程岗位简历", availability: "available" },
  { root_handle: "root-11111111111111111111111111111111", display_label: "外置盘历史简历", availability: "unavailable" },
]
const PREVIEW_ROOT_CONTROLS: Record<string, RootControlState> = {
  "root-00000000000000000000000000000000": "active",
  "root-11111111111111111111111111111111": "paused",
}

const STARTING_LIFECYCLE: DaemonLifecycleSnapshot = {
  schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
  state: "starting",
  generation: 0,
  restart_attempt: 0,
  restart_budget: 5,
  retry_delay_ms: null,
  consecutive_heartbeat_failures: 0,
  blocked_reason: null,
  last_exit: null,
  restart_ledger_reason: null,
}
const PREVIEW_LIFECYCLE: DaemonLifecycleSnapshot = { ...STARTING_LIFECYCLE, state: "ready", generation: 1 }

interface ResultSnapshot {
  generation: number
  visibleEpoch: number
}

function lifecycleMessage(snapshot: DaemonLifecycleSnapshot): string {
  if (snapshot.state === "starting") return "正在启动本地 daemon"
  if (snapshot.state === "recovering") return snapshot.retry_delay_ms === null
    ? "daemon 正在恢复"
    : `daemon 正在恢复，约 ${snapshot.retry_delay_ms}ms 后重试`
  if (snapshot.state === "circuit_open") return "daemon 连续异常，恢复电路暂时开路"
  if (snapshot.state === "blocked") return blockedReasonMessage(snapshot.blocked_reason)
  return "daemon 已就绪"
}

function lifecycleLabel(snapshot: DaemonLifecycleSnapshot, service: DaemonService): string {
  if (snapshot.state === "ready") {
    if (service === "ready") return "daemon 可用"
    if (service === "repairing") return "daemon 修复中"
    return "daemon 服务降级"
  }
  if (snapshot.state === "starting") return "daemon 启动中"
  if (snapshot.state === "recovering") return "daemon 恢复中"
  if (snapshot.state === "circuit_open") return "daemon 已开路"
  return "daemon 已阻止"
}

function Pill({ tone = "neutral", children }: { tone?: "neutral" | "ok" | "warn" | "err" | "info" | "primary"; children: ReactNode }) {
  return <span className={`pill pill-${tone}`}><span className="pill-dot" />{children}</span>
}

function Tag({ tone = "neutral", children }: { tone?: "neutral" | "ok" | "warn" | "primary"; children: ReactNode }) {
  return <span className={`tag tag-${tone}`}>{children}</span>
}

function SlideOver({ title, subtitle, onClose, children }: { title: string; subtitle?: string; onClose: () => void; children: ReactNode }) {
  return <div className="overlay" role="dialog" aria-modal="true" aria-label={title}>
    <button type="button" className="overlay-backdrop" aria-label="关闭" onClick={onClose} />
    <section className="sheet">
      <header className="sheet-header"><div><h2>{title}</h2>{subtitle && <p>{subtitle}</p>}</div><button type="button" className="icon-button" onClick={onClose} aria-label="关闭面板"><X size={16} /></button></header>
      {children}
    </section>
  </div>
}

function fileStem(fileName: string) {
  return fileName.replace(/\.[^.]+$/, "").replaceAll("_", " ")
}

function fileExtension(fileName: string) {
  return fileName.split(".").pop()?.toUpperCase() ?? "FILE"
}

function queryTerms(query: string) {
  return [...new Set(query.trim().split(/\s+/).filter(Boolean))].slice(0, 5)
}

function ResultCard({ hit, terms, onOpen, disabled }: { hit: SearchHit; terms: string[]; onOpen: () => void; disabled: boolean }) {
  return <button type="button" className="result-card" onClick={onOpen} disabled={disabled}>
    <div className="result-heading"><div className="result-title"><strong>{fileStem(hit.file_name)}</strong><span>本地简历</span></div><span className="result-state">{disabled ? "恢复后可查看" : "可搜索"}</span></div>
    <p>{hit.snippet || "（无命中摘要）"}</p>
    <div className="tag-row">{terms.map((term) => <Tag key={term}>{term}</Tag>)}</div>
    <div className="result-meta">
      <span><MapPin size={12} />本地索引</span>
      <span><FileText size={12} />{fileExtension(hit.file_name)}</span>
      <span>结果 #{hit.rank}</span>
      <span>正文与结构化字段</span>
      <span className="semantic-meta"><Sparkles size={12} />选择查看完整内容</span>
    </div>
  </button>
}

function countLabel(value: number | null | undefined): string {
  return value === null || value === undefined ? "—" : value.toLocaleString()
}

export function indexServicePresentation(
  service: DaemonService,
  repairReason: StatusBody["repair_reason"],
  repairProgress?: StatusBody["repair_progress"],
): { title: string; message: string } {
  if (service === "ready") return { title: "索引可用", message: "daemon 可用" }
  if (service === "repairing") {
    if (repairProgress?.phase === "retry_wait") {
      const retrySeconds = Math.ceil((repairProgress.retry_after_ms ?? 0) / 1000)
      return {
        title: "索引修复等待重试",
        message: `第 ${repairProgress.attempt ?? "—"}/${repairProgress.max_attempts ?? "—"} 次修复未完成，${retrySeconds} 秒后继续`,
      }
    }
    if (repairProgress?.phase === "rebuilding") {
      return {
        title: "索引修复中",
        message: `正在执行第 ${repairProgress.attempt ?? "—"}/${repairProgress.max_attempts ?? "—"} 次修复`,
      }
    }
    if (repairProgress?.phase === "migration_rebuild") {
      return { title: "索引升级中", message: "正在从本地元数据重建当前索引" }
    }
    return { title: "索引修复中", message: "daemon 已连接，索引正在修复" }
  }
  if (repairReason === "runtime_invariant") {
    return {
      title: "索引修复已阻塞",
      message: "daemon 已连接，索引修复已阻塞，请导出诊断",
    }
  }
  if (repairReason === "source_unavailable") {
    return {
      title: "索引修复已阻塞",
      message: "daemon 已连接，来源不可用，请恢复来源磁盘连接或文件权限",
    }
  }
  return { title: "索引能力降级", message: "daemon 已连接，索引能力降级" }
}

export function IndexServiceSummary({
  lifecycle,
  service,
  status,
  searchablePercent,
  connectionMessage,
}: {
  lifecycle: DaemonLifecycleSnapshot
  service: DaemonService
  status: Pick<StatusBody, "repair_reason" | "repair_progress" | "searchable_documents" | "ocr_queue_depth"> | null
  searchablePercent: number
  connectionMessage: string
}) {
  const presentation = indexServicePresentation(
    service,
    status?.repair_reason ?? null,
    status?.repair_progress ?? null,
  )
  const healthy = lifecycle.state === "ready" && service === "ready"
  const title = lifecycle.state === "ready"
    ? presentation.title
    : lifecycleLabel(lifecycle, service)
  const message = healthy && status
    ? `${countLabel(status.searchable_documents)} 份可搜索 · OCR 队列 ${countLabel(status.ocr_queue_depth)}`
    : lifecycle.state === "ready" ? presentation.message : connectionMessage
  return <>
    <div className="status-title"><strong>{title}</strong><span className={healthy ? "ok-text" : "warn-text"}>{searchablePercent}%</span></div>
    <progress className="progress-track" data-health={healthy ? "ok" : "warn"} value={searchablePercent} max={100} aria-label="可搜索简历比例" />
    <p>{message}</p>
  </>
}

function DiagnosticsContent({ state, message, diagnostics, onExport }: { state: DiagnosticsState; message: string; diagnostics: DiagnosticsBody | null; onExport: () => void }) {
  const privacySafe = diagnostics !== null && !diagnostics.contains_raw_resume_text && !diagnostics.contains_queries && !diagnostics.contains_resume_paths && !diagnostics.contains_candidate_results && !diagnostics.contains_snippet_text
  return <div className="sheet-scroll diagnostics-content">
    <div className={`banner banner-${state === "error" || state === "blocked" || state === "overload" ? "err" : state === "saved" ? "ok" : "neutral"}`} aria-live="polite">
      {state === "loading" || state === "exporting" ? <LoaderCircle className="spin" size={16} /> : state === "error" || state === "blocked" || state === "overload" ? <AlertTriangle size={16} /> : <ShieldCheck size={16} />}<span>{message}</span>
    </div>
    {diagnostics && <>
      <section className="panel-card"><header><strong>脱敏导出边界</strong><Pill tone={privacySafe ? "ok" : "err"}>{privacySafe ? "5/5 通过" : "阻止导出"}</Pill></header>
        {["简历正文", "查询文本", "原始路径", "候选结果", "结果摘要"].map((label) => <div className="check-row" key={label}><CheckCircle2 size={14} /><span>{label}</span><small>不包含</small></div>)}
      </section>
      <section className="panel-card"><header><strong>本地聚合</strong><span>{diagnostics.evidence_lane} · {diagnostics.evidence_status} · epoch {countLabel(diagnostics.visible_epoch)}</span></header><dl>
        <div><dt>已索引 / 可搜索</dt><dd>{countLabel(diagnostics.metrics.indexed_documents)} / {countLabel(diagnostics.metrics.searchable_documents)}</dd></div>
        <div><dt>OCR / embedding</dt><dd>{countLabel(diagnostics.metrics.ocr_queue_depth)} / {countLabel(diagnostics.metrics.embedding_queue_depth)}</dd></div>
        <div><dt>可恢复 / 永久失败</dt><dd>{countLabel(diagnostics.error_counts.failed_retryable)} / {countLabel(diagnostics.error_counts.failed_permanent)}</dd></div>
        <div><dt>查询 P95</dt><dd>{diagnostics.metrics.query_latency?.p95_ms === null || diagnostics.metrics.query_latency === null ? "—" : `${diagnostics.metrics.query_latency.p95_ms}ms`}</dd></div>
      </dl></section>
      <button className="primary-button wide-button" onClick={onExport} disabled={!privacySafe || state === "exporting" || state === "loading"}>{state === "exporting" ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}导出脱敏 JSON</button>
    </>}
  </div>
}

export function App() {
  const previewMode = import.meta.env.DEV ? new URLSearchParams(window.location.search).get("preview") : null
  const preview = previewMode === "search" || previewMode === "detail" || previewMode === "import"
  const [lifecycle, setLifecycle] = useState<DaemonLifecycleSnapshot>(preview ? PREVIEW_LIFECYCLE : STARTING_LIFECYCLE)
  const [service, setService] = useState<DaemonService>(preview ? "ready" : "degraded")
  const [resultFreshness, setResultFreshness] = useState<ResultFreshness>("current")
  const [connectionMessage, setConnectionMessage] = useState(preview ? "daemon 可用" : lifecycleMessage(STARTING_LIFECYCLE))
  const [status, setStatus] = useState<StatusBody | null>(preview ? { schema_version: "daemon.status.v2", status: "ok", process_state: "ready", service_state: "ready", services: { metadata: "ready", query: "ready" }, repair_reason: null, repair_progress: null, error: null, indexed_documents: 1284, searchable_documents: 1098, partial_documents: 84, visible_epoch: 1, failed_retryable: 2, failed_permanent: 1, recovery_queue_depth: 0, ocr_queue_depth: 102, embedding_queue_depth: 186, entity_mentions: 0, import_tasks_queued: 0, index_health: "ready", latest_import_scan: { files_discovered: 1284, searchable_documents: 1098, ocr_required_documents: 102, failed_documents: 1 }, ipc: { accepted: 8, completed: 8, client_disconnect: 0, request_failure: 0, response_failure: 0 } } : null)
  const [query, setQuery] = useState(preview ? "Java Kafka 支付" : "")
  const [mode, setMode] = useState<Mode>(preview ? "hybrid" : "keyword")
  const [showFilters, setShowFilters] = useState(false)
  const [skills, setSkills] = useState("")
  const [location, setLocation] = useState("")
  const [degree, setDegree] = useState<Degree>("")
  const [years, setYears] = useState("")
  const [view, setView] = useState<ViewState>(preview ? "complete" : "idle")
  const [message, setMessage] = useState(preview ? "命中 5 条" : "输入关键词开始本地检索")
  const [results, setResults] = useState<SearchHit[]>(preview ? PREVIEW_RESULTS : [])
  const [resultPage, setResultPage] = useState(0)
  const [latency, setLatency] = useState<number | null>(preview ? 42 : null)
  const [overlay, setOverlay] = useState<Overlay>(previewMode === "import" ? "import" : null)
  const [managedRoots, setManagedRoots] = useState<ManagedRoot[]>(previewMode === "import" ? PREVIEW_MANAGED_ROOTS : [])
  const [rootControls, setRootControls] = useState<Record<string, RootControlState>>(previewMode === "import" ? PREVIEW_ROOT_CONTROLS : {})
  const [selectedRoot, setSelectedRoot] = useState<ManagedRoot | null>(previewMode === "import" ? PREVIEW_MANAGED_ROOTS[0] : null)
  const [importState, setImportState] = useState<ImportState>(previewMode === "import" ? "selected" : "idle")
  const [importMessage, setImportMessage] = useState(previewMode === "import" ? "已恢复 2 个本地授权目录" : "选择一个本地目录后提交完整扫描")
  const [diagnosticsState, setDiagnosticsState] = useState<DiagnosticsState>("idle")
  const [diagnosticsMessage, setDiagnosticsMessage] = useState("尚未读取本地脱敏诊断")
  const [diagnostics, setDiagnostics] = useState<DiagnosticsBody | null>(null)
  const cancelToken = useRef<string | null>(null)
  const previewDetailOpened = useRef(false)
  const lifecycleRef = useRef(lifecycle)
  const resultSnapshot = useRef<ResultSnapshot | null>(preview ? { generation: 1, visibleEpoch: 1 } : null)
  const managedRootsGeneration = useRef<number | null>(null)
  const {
    detail,
    detailLoading,
    detailError,
    fullText,
    bodyComplete,
    detailInterrupted,
    open: openDetail,
    resume: resumeDetail,
    reset: resetDetail,
    observeLifecycle: observeDetailLifecycle,
  } = useDetailSession({ preview, lifecycleRef, service, onStaleSelection: () => setResultFreshness("stale") })

  const terms = useMemo(() => queryTerms(query), [query])
  const filterCount = [skills, location, degree, years].filter((value) => value.trim()).length
  const resultPageCount = Math.max(1, Math.ceil(results.length / RESULT_PAGE_SIZE))
  const visibleResults = results.slice(resultPage * RESULT_PAGE_SIZE, (resultPage + 1) * RESULT_PAGE_SIZE)
  const latestScan = status?.latest_import_scan
  const searchablePercent = status?.indexed_documents && status.searchable_documents !== null ? Math.round((status.searchable_documents / status.indexed_documents) * 100) : 0
  const health = lifecycle.state === "ready"
    ? service === "ready" ? "ok" : "degraded"
    : lifecycle.state === "starting" || lifecycle.state === "recovering" ? "loading" : "unavailable"
  const operationsPaused = lifecycle.state !== "ready" || service !== "ready"

  async function refreshStatus() {
    if (preview) return
    try {
      const reply = await readStatus()
      const body = reply.body.schema_version === "daemon.status.v2" ? reply.body : null
      setStatus(body)
      const nextService = serviceStateFromStatus({
        httpStatus: reply.http_status,
        status: body?.service_state ?? "unavailable",
      })
      setService(nextService)
      const result = resultSnapshot.current
      if (body) {
        setResultFreshness((current) => reconcileResultFreshness({
          current,
          hasResults: result !== null,
          resultGeneration: result?.generation ?? null,
          resultVisibleEpoch: result?.visibleEpoch ?? null,
          lifecycle: lifecycleRef.current,
          serviceVisibleEpoch: body.visible_epoch,
        }))
      }
      setConnectionMessage(indexServicePresentation(
        nextService,
        body?.repair_reason ?? null,
      ).message)
    } catch (error) {
      if (bridgeFailureKind(error) === "overload") {
        setConnectionMessage("状态刷新繁忙，稍后自动重试")
        return
      }
      setStatus(null)
      setService("degraded")
      setConnectionMessage("本地 daemon 状态暂时不可读")
    }
  }

  function applyLifecycleSnapshot(snapshot: DaemonLifecycleSnapshot) {
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
    observeDetailLifecycle(snapshot)
    if (snapshot.state !== "ready") {
      setService("degraded")
      setConnectionMessage(lifecycleMessage(snapshot))
    }
  }

  async function retryLifecycle() {
    if (preview) return
    setConnectionMessage("正在请求 daemon 监督器重试")
    try {
      const snapshot = await retryDaemon()
      if (!isDaemonLifecycleSnapshot(snapshot)) throw new Error("lifecycle contract mismatch")
      applyLifecycleSnapshot(snapshot)
    } catch (error) {
      if (bridgeFailureKind(error) === "overload") {
        setConnectionMessage("daemon 监督器正在处理另一个请求")
        return
      }
      setConnectionMessage(bridgeError(error).message)
    }
  }

  async function refreshManagedRoots() {
    if (preview) return
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
      next[root.root_handle] = "loading"
      setRootControls({ ...next })
      try {
        const outcome = managedRootControlOutcome(await controlManagedRoot(root.root_handle, "inspect"))
        next[root.root_handle] = outcome
      } catch (error) {
        next[root.root_handle] = bridgeFailureKind(error) === "overload" ? "overload" : "error"
      }
    }
    setRootControls(next)
  }

  useEffect(() => {
    if (preview) return
    return startSerialLifecyclePolling({
      readSnapshot: getDaemonLifecycle,
      onSnapshot: async (snapshot) => {
        if (!isDaemonLifecycleSnapshot(snapshot)) throw new Error("lifecycle contract mismatch")
        applyLifecycleSnapshot(snapshot)
        if (snapshot.state === "ready") {
          await refreshStatus()
          if (managedRootsGeneration.current !== snapshot.generation) {
            managedRootsGeneration.current = snapshot.generation
            await refreshManagedRoots()
          }
        }
      },
      onError: (error) => setConnectionMessage(bridgeError(error).message),
      clock: {
        setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
        clearTimeout: (timer) => window.clearTimeout(timer),
      },
      focusEvents: {
        addFocusListener: (listener) => window.addEventListener("focus", listener),
        removeFocusListener: (listener) => window.removeEventListener("focus", listener),
      },
    })
  }, [preview])
  useEffect(() => {
    if (previewMode !== "detail" || previewDetailOpened.current) return
    previewDetailOpened.current = true
    void openDetail(PREVIEW_RESULTS[0])
  }, [previewMode])

  async function runSearch(event: FormEvent) {
    event.preventDefault()
    if (!query.trim() || view === "loading" || lifecycleRef.current.state !== "ready" || service !== "ready") return
    if (preview) { setView("complete"); setResults(PREVIEW_RESULTS); setResultPage(0); setLatency(42); setResultFreshness("current"); setMessage("命中 5 条"); return }
    const id = crypto.randomUUID()
    const startedGeneration = lifecycleRef.current.generation
    const previousView = view
    cancelToken.current = `gui-cancel-${id}`
    resetDetail()
    setView("loading"); setMessage("正在检索")
    const filters = {
      ...(skills.trim() ? { skills_any: skills.split(/[,，\s]+/).filter(Boolean) } : {}),
      ...(location.trim() ? { locations_any: [location.trim()] } : {}),
      ...(degree ? { degree_min: degree } : {}),
      ...(years && Number.isFinite(Number(years)) ? { years_experience_min: Number(years) } : {}),
    }
    try {
      const reply = await searchResumes({
        schema_version: "resume-ir.ipc-request.v3", request_id: `gui-search-${id}`, client_capability: "interactive_gui", deadline_ms: searchDeadlineMs(mode), cancel_token: cancelToken.current,
        payload: { query, mode: mode === "field" ? "fulltext" : mode === "keyword" ? "fulltext" : mode, top_k: 50, filters },
      })
      if (!detailReadMayContinue(lifecycleRef.current, startedGeneration)) {
        setResultFreshness("interrupted")
        setView(results.length > 0 ? previousView : "error")
        setMessage("daemon 已换代，本次搜索已中断；结果未自动重放")
        return
      }
      const body = reply.body
      const outcome = searchOutcome(reply)
      if (body.schema_version === "daemon.error.v1") {
        resultSnapshot.current = null; setResultFreshness("current"); setResults([]); setView("error"); setMessage(`查询失败：${body.status}`); return
      }
      if (body.schema_version === "resume-ir.error.v1") {
        if (body.error.code === "REPAIRING" || body.error.code === "METADATA_UNAVAILABLE" || body.error.code === "QUERY_SERVICE_UNAVAILABLE") {
          setService(body.error.code === "REPAIRING" ? "repairing" : "degraded")
          setResultFreshness(results.length > 0 ? "interrupted" : "current")
          setView(results.length > 0 ? previousView : "error")
          setMessage(body.error.code === "REPAIRING" ? "索引正在修复；现有结果已保留" : "查询服务暂时不可用；现有结果已保留")
        } else if (body.error.code === "OVERLOADED") {
          setResultFreshness("current")
          setView(results.length > 0 ? previousView : "overload")
          setMessage(`查询负载已满，建议 ${body.error.retry_after_ms ?? 250}ms 后重试`)
        } else {
          setResultFreshness("current")
          setView(results.length > 0 ? previousView : "error")
          setMessage(body.error.code === "SEMANTIC_DISABLED" ? "本地语义能力未启用，请显式选择关键词或字段模式" : `查询失败：${body.error.code}`)
        }
        return
      }
      resultSnapshot.current = body.results.length > 0 ? { generation: startedGeneration, visibleEpoch: body.visible_epoch } : null
      setResultFreshness("current")
      setLatency(body.latency_ms); setResults(body.results); setResultPage(0)
      if (outcome === "overload") { setView("overload"); setMessage("查询负载已满") }
      else if (outcome === "cancelled") { setView("cancelled"); setMessage("本次查询已取消") }
      else if (outcome === "error") { setView("error"); setMessage("查询失败") }
      else if (outcome === "partial") { setView("partial"); setMessage(`部分结果：${body.partial_reasons.join("、") || "能力降级"}`) }
      else if (outcome === "empty") { setView("empty"); setMessage("没有简历同时满足当前条件") }
      else { setView("complete"); setMessage(`命中 ${body.result_count} 条`) }
    } catch (error) {
      const failure = bridgeFailureKind(error)
      if (failure === "unavailable" || !detailReadMayContinue(lifecycleRef.current, startedGeneration)) {
        setResultFreshness(results.length > 0 ? "interrupted" : "current")
        setView(results.length > 0 ? previousView : "error")
        setMessage("daemon 恢复打断了本次搜索；现有结果已保留且不会自动重放")
      } else {
        resultSnapshot.current = null
        setResultFreshness("current")
        setResults([])
        setView(failure === "overload" ? "overload" : "error")
        setMessage(failure === "overload" ? "桌面查询入口繁忙，请稍后重试" : bridgeError(error).message)
      }
    }
    finally { cancelToken.current = null }
  }

  async function cancelSearch() {
    const token = cancelToken.current
    if (!token) return
    await requestSearchCancel(`gui-cancel-command-${crypto.randomUUID()}`, token).catch((error) => {
      setMessage(bridgeFailureKind(error) === "overload" ? "取消入口繁忙，本次查询仍在执行" : "取消请求未送达，本次查询仍在执行")
    })
  }

  async function chooseImportRoot() {
    setImportState("selecting"); setImportMessage("正在打开本机目录选择器")
    try { const root = await selectImportRoot(); if (!root) { setImportState("cancelled"); setImportMessage("未选择目录"); return } const selected = { ...root, availability: "available" as const }; setSelectedRoot(selected); await refreshManagedRoots(); setSelectedRoot(selected); setImportState("selected"); setImportMessage("目录已持久授权，可提交完整扫描") }
    catch (error) { const overload = bridgeFailureKind(error) === "overload"; setSelectedRoot(null); setImportState(overload ? "overload" : "error"); setImportMessage(overload ? "目录选择入口繁忙，请稍后重试" : bridgeError(error).message) }
  }

  async function requestRootScan(root: ManagedRoot, intent: "initial" | "rescan") {
    if (root.availability !== "available") {
      setImportState("error"); setImportMessage("目录当前不可读取，请恢复磁盘或权限"); return
    }
    if (rootControls[root.root_handle] === "paused") {
      setImportState("error"); setImportMessage("目录监控已暂停，请先恢复监控"); return
    }
    if (preview) {
      setSelectedRoot(root); setImportState("queued"); setImportMessage(intent === "rescan" ? "已开始增量重新扫描" : "已创建本地导入任务"); return
    }
    setSelectedRoot(root); setImportState("submitting"); setImportMessage(intent === "rescan" ? "正在提交增量重新扫描" : "正在提交本地导入任务")
    try {
      const reply = intent === "rescan" ? await rescanManagedRoot(root.root_handle) : await importSelectedRoot(root.root_handle)
      const outcome = managedRootScanOutcome(reply)
      setImportState(outcome)
      if (outcome === "queued") setImportMessage(intent === "rescan" ? "已开始增量重新扫描" : "已创建本地导入任务")
      else if (outcome === "pending") setImportMessage("该目录已有待处理扫描任务")
      else if (outcome === "active") setImportMessage("该目录正在扫描，无需重复提交")
      else { setImportMessage("daemon 未接受目录扫描任务"); return }
      await refreshStatus()
    } catch (error) {
      const overload = bridgeFailureKind(error) === "overload"
      setImportState(overload ? "overload" : "error")
      setImportMessage(overload ? "目录扫描入口繁忙，请稍后重试" : bridgeError(error).message)
    }
  }

  async function changeRootControl(root: ManagedRoot, action: ManagedRootControlAction) {
    if (operationsPaused) return
    if (action === "resume" && root.availability !== "available") {
      setImportState("unavailable")
      setImportMessage("目录当前不可读取，重新授权后才能恢复监控")
      return
    }
    setSelectedRoot(root)
    setRootControls((current) => ({ ...current, [root.root_handle]: "loading" }))
    if (preview) {
      const state = action === "pause" ? "paused" : "active"
      setRootControls((current) => ({ ...current, [root.root_handle]: state }))
      setImportState("selected")
      setImportMessage(state === "paused" ? "已暂停此目录的监听与周期扫描" : "已恢复监控，并开始追赶目录变更")
      return
    }
    try {
      const reply = await controlManagedRoot(root.root_handle, action)
      const outcome = managedRootControlOutcome(reply)
      setRootControls((current) => ({ ...current, [root.root_handle]: outcome }))
      if (outcome === "unmanaged") {
        setImportState("selected")
        setImportMessage("目录已授权；完成首次扫描后可暂停或恢复持续监控")
        return
      }
      if (outcome === "error") {
        setImportState("error")
        setImportMessage("daemon 未接受目录监控操作，可重试读取状态")
        return
      }
      setImportState("selected")
      const body = reply.body.schema_version === "daemon.import_root_control.v1" ? reply.body : null
      if (action === "pause") setImportMessage(body?.task_cancel_requested ? "已暂停监控，并请求取消此目录的活动任务" : "已暂停此目录的监听与周期扫描")
      else if (action === "resume") setImportMessage(body?.catch_up_queued ? "已恢复监控，并开始追赶目录变更" : "目录监控已恢复，无需重复追赶")
      else setImportMessage(outcome === "paused" ? "目录监控保持暂停" : "目录持续监控正常")
      await refreshStatus()
    } catch (error) {
      const state = bridgeFailureKind(error) === "overload" ? "overload" : "error"
      setRootControls((current) => ({ ...current, [root.root_handle]: state }))
      setImportState(state)
      setImportMessage(state === "overload" ? "目录监控入口繁忙，请稍后重试" : bridgeError(error).message)
    }
  }

  async function reauthorizeRoot(root: ManagedRoot) {
    if (root.availability !== "unavailable") return
    setSelectedRoot(root)
    setImportState("reauthorizing")
    setImportMessage("正在打开原目录重新授权选择器")
    if (preview) {
      const restored = { ...root, availability: "available" as const }
      setManagedRoots((current) => current.map((candidate) => candidate.root_handle === root.root_handle ? restored : candidate))
      setSelectedRoot(restored)
      setImportState("selected")
      setImportMessage("原目录权限已恢复，可重新扫描")
      return
    }
    try {
      const restored = await reauthorizeManagedRoot(root.root_handle)
      if (!restored) {
        setImportState("cancelled")
        setImportMessage("已取消重新授权，原授权记录保持不变")
        return
      }
      if (restored.root_handle !== root.root_handle) {
        setImportState("mismatch")
        setImportMessage("重新授权返回了不一致的目录身份，原授权记录保持不变")
        return
      }
      const availableRoot = { ...restored, availability: "available" as const }
      setManagedRoots((current) => current.map((candidate) => candidate.root_handle === root.root_handle ? availableRoot : candidate))
      await refreshManagedRoots()
      setSelectedRoot(availableRoot)
      setImportState("selected")
      setImportMessage("原目录权限已恢复，可重新扫描")
    } catch (error) {
      const failure = managedRootRecoveryFailure(error)
      setImportState(failure)
      if (failure === "overload") setImportMessage("重新授权入口繁忙，请稍后重试")
      else if (failure === "mismatch") setImportMessage("所选目录与待恢复授权不一致，原授权记录保持不变")
      else if (failure === "unavailable") setImportMessage("所选原目录当前仍不可读取，请恢复磁盘或权限后重试")
      else setImportMessage(bridgeError(error).message)
    }
  }

  async function submitImport() {
    if (selectedRoot) await requestRootScan(selectedRoot, "initial")
  }

  async function openDiagnostics() {
    resetDetail(); setOverlay("diagnostics"); setDiagnosticsState("loading"); setDiagnosticsMessage("正在读取本地聚合诊断")
    try { const reply = await readDiagnostics(); if (reply.http_status !== 200 || reply.body.schema_version !== "resume-ir.diagnostics.v3" || reply.body.privacy_boundary !== "redacted_local_aggregate") { setDiagnostics(null); setDiagnosticsState("blocked"); setDiagnosticsMessage("诊断合同未满足脱敏导出边界"); return } setDiagnostics(reply.body); setDiagnosticsState("ready"); setDiagnosticsMessage("只读聚合诊断已就绪") }
    catch (error) { const overload = bridgeFailureKind(error) === "overload"; setDiagnostics(null); setDiagnosticsState(overload ? "overload" : "error"); setDiagnosticsMessage(overload ? "诊断读取入口繁忙，请稍后重试" : bridgeError(error).message) }
  }

  async function saveDiagnostics() {
    if (!diagnostics) return
    setDiagnosticsState("exporting"); setDiagnosticsMessage("正在打开保存位置选择器")
    try { const receipt = await exportDiagnostics(); if (!receipt) { setDiagnosticsState("cancelled"); setDiagnosticsMessage("已取消导出"); return } setDiagnosticsState("saved"); setDiagnosticsMessage(`已导出 ${receipt.file_label}`) }
    catch (error) { const overload = bridgeFailureKind(error) === "overload"; setDiagnosticsState(overload ? "overload" : "error"); setDiagnosticsMessage(overload ? "诊断导出入口繁忙，请稍后重试" : bridgeError(error).message) }
  }

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span>IR</span><div><strong>resume-ir</strong><ChevronRight size={13} /></div></div>
      <button className="sidebar-search" onClick={() => document.getElementById("query")?.focus()}><Search size={14} /><span>搜索简历…</span><kbd>⌘K</kbd></button>
      <nav aria-label="主导航">
        <button className="nav-active" onClick={() => { setOverlay(null); resetDetail() }}><Search size={16} />搜索</button>
        <button onClick={() => { resetDetail(); setOverlay("import") }}><FolderTree size={16} />简历来源</button>
        <div className="nav-label">系统</div>
        <button onClick={() => void openDiagnostics()}><ShieldCheck size={16} />隐私与诊断</button>
      </nav>
      <div className="sidebar-status">
        <IndexServiceSummary lifecycle={lifecycle} service={service} status={status} searchablePercent={searchablePercent} connectionMessage={connectionMessage} />
        {(lifecycle.state === "circuit_open" || lifecycle.state === "blocked") && <button type="button" className="plain-button wide-button" onClick={() => void retryLifecycle()}>请求监督器重试</button>}
        <div className="local-only"><HardDriveDownload size={14} />完全本地运行 · 不上传</div>
      </div>
    </aside>

    <main className="main-shell">
      <header className="topbar"><div><span>resume-ir</span><ChevronRight size={14} /><strong>搜索</strong></div><Pill tone={health === "ok" ? "ok" : lifecycle.state === "blocked" ? "err" : "warn"}>{lifecycleLabel(lifecycle, service)}</Pill></header>
      <form className="search-head" onSubmit={runSearch}>
        <div className="query-box"><Search size={16} /><input id="query" value={query} onChange={(event) => setQuery(event.target.value)} maxLength={512} placeholder="输入关键词，空格分隔多个 Query（默认 AND 交集）" />{query && <button type="button" className="icon-button" aria-label="清空" onClick={() => setQuery("")}><X size={16} /></button>}</div>
        <div className="search-controls">
          <div className="term-chain">{terms.length > 1 && terms.map((term, index) => <span key={term}>{index > 0 && <b>AND</b>}<Tag tone="primary">{term}</Tag></span>)}</div>
          <div className="control-actions"><div className="segmented">{MODE_OPTIONS.map((option) => <button type="button" key={option.value} className={mode === option.value ? "selected" : ""} onClick={() => { setMode(option.value); if (option.value === "field") setShowFilters(true) }}>{option.label}</button>)}</div><button type="button" className={showFilters ? "filter-button active" : "filter-button"} onClick={() => setShowFilters((open) => !open)}><SlidersHorizontal size={14} />过滤{filterCount ? ` · ${filterCount}` : ""}</button><button className="primary-button" type="submit" disabled={health !== "ok" || !query.trim() || view === "loading"}>{view === "loading" ? <LoaderCircle className="spin" size={15} /> : <Search size={15} />}搜索</button>{view === "loading" && <button type="button" className="plain-button" onClick={() => void cancelSearch()}>取消</button>}</div>
        </div>
      </form>

      <div className="content-shell">
        <section className="results-pane" aria-live="polite">
          {view !== "idle" && <div className={`execution-bar execution-${view}`}><div>{view === "loading" ? <LoaderCircle className="spin" size={15} /> : view === "error" || view === "overload" ? <AlertTriangle size={15} /> : <Pill tone={view === "partial" ? "warn" : view === "complete" ? "ok" : "neutral"}>{view === "partial" ? "部分结果" : view === "complete" ? "搜索完成" : "搜索状态"}</Pill>}<span>{message}</span>{latency !== null && <span><Clock3 size={14} />{latency.toFixed(0)} ms</span>}<span>已索引 {countLabel(status?.searchable_documents)} / {countLabel(status?.indexed_documents)}</span></div><div><Tag tone={mode === "semantic" || mode === "hybrid" ? "ok" : "neutral"}>语义</Tag><Tag>正文</Tag>{filterCount > 0 && <Tag tone="primary">字段 · {filterCount}</Tag>}</div></div>}
          {view === "idle" && <div className="empty-state"><Search size={32} /><p>请输入搜索条件。空查询不会执行重型搜索。</p></div>}
          {(view === "empty" || view === "error" || view === "overload" || view === "cancelled") && <div className={`state-banner state-${view}`}><strong>{message}</strong><span>系统不会自动放宽查询语义。</span></div>}
          {resultFreshness === "interrupted" && results.length > 0 && <div className="state-banner"><strong>daemon 恢复打断了当前会话</strong><span>现有结果仅保留作上下文；系统不会自动重放搜索或详情请求。</span></div>}
          {resultFreshness === "stale" && results.length > 0 && <div className="state-banner"><strong>当前排序可能已更新</strong><span>结果不会自动重搜；详情仍由服务端按精确版本验证。</span></div>}
          <div className="result-list">{visibleResults.map((hit) => <ResultCard key={`${hit.selection.doc_id}:${hit.selection.version_id}`} hit={hit} terms={terms} onOpen={() => void openDetail(hit)} disabled={operationsPaused} />)}</div>
          {results.length > RESULT_PAGE_SIZE && <nav className="pagination" aria-label="搜索结果分页"><button type="button" className="plain-button" disabled={resultPage === 0} onClick={() => setResultPage((page) => Math.max(0, page - 1))}><ChevronLeft size={14} />上一页</button><span>第 {resultPage + 1} / {resultPageCount} 页</span><button type="button" className="plain-button" disabled={resultPage + 1 >= resultPageCount} onClick={() => setResultPage((page) => Math.min(resultPageCount - 1, page + 1))}>下一页<ChevronRight size={14} /></button></nav>}
        </section>
        {showFilters && <aside className="filter-panel"><div className="filter-title"><SlidersHorizontal size={16} /><strong>结构化字段过滤</strong><button className="icon-button" onClick={() => setShowFilters(false)}><X size={15} /></button></div><p>过滤条件与关键词为 AND 关系。留空表示不限。</p><label>技能（空格或逗号分隔）<input value={skills} onChange={(event) => setSkills(event.target.value)} placeholder="Java, Kafka" /></label><label>地点<input value={location} onChange={(event) => setLocation(event.target.value)} placeholder="上海" /></label><label>最低学历<select value={degree} onChange={(event) => setDegree(event.target.value as Degree)}><option value="">不限</option><option value="associate">大专</option><option value="bachelor">本科</option><option value="master">硕士</option><option value="doctorate">博士</option></select></label><label>最低工作年限<input value={years} onChange={(event) => setYears(event.target.value)} inputMode="decimal" placeholder="5" /></label>{filterCount > 0 && <button className="plain-button clear-filters" onClick={() => { setSkills(""); setLocation(""); setDegree(""); setYears("") }}>清除全部</button>}</aside>}
      </div>
    </main>

    {(detail || detailLoading || detailError) && <SlideOver title={detail ? fileStem(detail.file_name) : "简历详情"} subtitle={detail ? `${fileExtension(detail.file_name)} · ${Math.ceil(detail.source_byte_size / 1024)} KiB` : "正在读取本地详情"} onClose={resetDetail}>
      <div className="sheet-scroll detail-content">
        {detailLoading && !detail && <div className="detail-loading"><LoaderCircle className="spin" size={20} />正在读取精确版本的结构化字段与正文</div>}
        {detailError && <div className="banner banner-err"><AlertTriangle size={16} />{detailError}</div>}
        {detailInterrupted && lifecycle.state === "ready" && <button type="button" className="plain-button wide-button" onClick={() => void resumeDetail()} disabled={detailLoading || service === "repairing"}>显式续读当前版本</button>}
        {detail && <>
          <div className="status-row"><Pill tone="ok">精确版本</Pill><Pill tone="info">{detail.schema_version}</Pill>{bodyComplete ? <Pill tone="ok">正文完整</Pill> : detailInterrupted ? <Pill tone="warn">正文已中断</Pill> : <Pill tone="warn">正文读取中</Pill>}</div>
          <section className="detail-section"><h3>搜索命中摘要</h3><p className="snippet-box">{detail.snippet || "（无命中摘要）"}</p><div className="tag-row">{terms.map((term) => <Tag key={term} tone="primary">命中：{term}</Tag>)}</div></section>
          <section className="detail-section"><h3>提取字段</h3><dl className="field-grid">{detail.fields.slice(0, 32).map((field, index) => <div key={`${field.type}-${index}`}><dt>{field.type}</dt><dd>{field.value}<small>{Math.round(field.confidence * 100)}%</small></dd></div>)}</dl>{detail.fields_truncated && <small className="muted-note">字段已按本地响应上限截断</small>}</section>
          <section className="file-panel"><div><span>文件类型</span><strong>{fileExtension(detail.file_name)}</strong></div><div><span>来源大小</span><strong>{Math.ceil(detail.source_byte_size / 1024)} KiB</strong></div><div><span>解析合同</span><code>{detail.parse_version} · {detail.schema_version}</code></div><div><span>语言 / 页数</span><strong>{detail.language_set.join("、") || "—"} · {detail.page_count ?? "—"}</strong></div></section>
          <section className="detail-section"><h3>规范化简历全文</h3><pre className="full-text">{fullText || (detailLoading ? "正在读取…" : "（正文为空）")}</pre>{!bodyComplete && fullText && <small className="muted-note">{detailInterrupted ? "正文读取已中断；现有内容保持不变。" : detailLoading ? "正在继续读取同一版本正文…" : `正文超过桌面展示上限，已显示前 ${MAX_DETAIL_PAGES} 页。`}</small>}</section>
        </>}
      </div>
    </SlideOver>}

    {overlay === "import" && <SlideOver title="简历来源" subtitle="本地目录只由原生进程持有" onClose={() => setOverlay(null)}>
      <div className="sheet-scroll import-content">
        <div className={`banner banner-${["error", "mismatch", "overload"].includes(importState) ? "err" : ["queued", "pending", "active"].includes(importState) ? "ok" : "neutral"}`} aria-live="polite">
          {["selecting", "submitting", "reauthorizing"].includes(importState) ? <LoaderCircle className="spin" size={16} /> : ["error", "mismatch", "overload", "unavailable"].includes(importState) ? <AlertTriangle size={16} /> : <FolderOpen size={16} />}
          <span>{importMessage}</span>
        </div>
        {managedRoots.length > 0 ? <section className="panel-card">
          <header><strong>已授权目录</strong><span>{managedRoots.length} / 16</span></header>
          {managedRoots.map((root) => {
            const control = rootControls[root.root_handle] ?? "loading"
            const unavailable = root.availability === "unavailable"
            const description = unavailable ? "目录不可用 · 授权记录仍保留" : control === "paused" ? "监控已暂停 · 不会监听或周期扫描" : control === "active" ? "持续监控中 · 自动追赶目录变更" : control === "unmanaged" ? "已授权 · 首次扫描后开始持续监控" : control === "loading" ? "正在读取监控状态" : "监控状态暂不可用 · 可重试"
            const label = unavailable ? "不可用" : control === "active" ? "监控中" : control === "paused" ? "已暂停" : control === "unmanaged" ? "待首次扫描" : control === "loading" ? "读取中" : "状态未知"
            const tone = unavailable || control === "paused" ? "warn" : control === "active" ? "ok" : control === "error" || control === "overload" ? "err" : "neutral"
            const busy = control === "loading" || ["selecting", "submitting", "reauthorizing"].includes(importState)
            return <article className="source-card" key={root.root_handle}>
              <FolderTree size={24} />
              <div className="source-copy"><strong>{root.display_label}</strong><p>{description}</p></div>
              <Pill tone={tone}>{label}</Pill>
              <div className="source-actions">
                {unavailable ? <button type="button" className="plain-button" onClick={() => void reauthorizeRoot(root)} disabled={busy}>
                  <RefreshCw size={14} />{importState === "reauthorizing" && selectedRoot?.root_handle === root.root_handle ? "授权中" : "重新授权"}
                </button> : control !== "paused" && <button type="button" className="plain-button" onClick={() => void requestRootScan(root, "rescan")} disabled={health !== "ok" || busy}>
                  <RefreshCw size={14} />{importState === "submitting" && selectedRoot?.root_handle === root.root_handle ? "提交中" : control === "unmanaged" ? "开始扫描" : "重新扫描"}
                </button>}
                {control === "active" && <button type="button" className="plain-button" onClick={() => void changeRootControl(root, "pause")} disabled={operationsPaused || busy}><Pause size={14} />暂停监控</button>}
                {control === "paused" && <button type="button" className="plain-button" onClick={() => void changeRootControl(root, "resume")} disabled={operationsPaused || busy || unavailable}><Play size={14} />恢复监控</button>}
                {(control === "overload" || control === "error") && <button type="button" className="plain-button" onClick={() => void changeRootControl(root, "inspect")} disabled={operationsPaused || busy}><RefreshCw size={14} />重试状态</button>}
              </div>
            </article>
          })}
        </section> : <section className="source-card"><FolderTree size={24} /><div><strong>尚未选择目录</strong><p>目录扫描、解析、分类与索引全部在本机完成。</p></div></section>}
        <div className="sheet-actions">
          <button type="button" className="plain-button" onClick={() => void chooseImportRoot()} disabled={["selecting", "submitting", "reauthorizing"].includes(importState)}><FolderOpen size={15} />{managedRoots.length > 0 ? "添加目录" : "选择目录"}</button>
          {selectedRoot && <button type="button" className="primary-button" onClick={() => void submitImport()} disabled={selectedRoot.availability !== "available" || rootControls[selectedRoot.root_handle] === "paused" || health !== "ok" || ["submitting", "reauthorizing"].includes(importState)}><Upload size={15} />{selectedRoot.availability !== "available" ? "目录不可用" : rootControls[selectedRoot.root_handle] === "paused" ? "监控已暂停" : "扫描此目录"}</button>}
        </div>
        <section className="panel-card source-summary"><header><strong>当前本地索引</strong></header><dl><div><dt>已发现</dt><dd>{latestScan?.files_discovered ?? "—"}</dd></div><div><dt>可搜索</dt><dd>{status?.searchable_documents ?? "—"}</dd></div><div><dt>OCR 待处理</dt><dd>{status?.ocr_queue_depth ?? "—"}</dd></div><div><dt>失败</dt><dd>{latestScan?.failed_documents ?? "—"}</dd></div></dl></section>
      </div>
    </SlideOver>}
    {overlay === "diagnostics" && <SlideOver title="隐私与诊断" subtitle="敏感详情可在本地展示；导出证据仍保持脱敏" onClose={() => setOverlay(null)}><DiagnosticsContent state={diagnosticsState} message={diagnosticsMessage} diagnostics={diagnostics} onExport={() => void saveDiagnostics()} /></SlideOver>}
  </div>
}
