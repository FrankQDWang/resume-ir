import { type FormEvent, type ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Clock3,
  FileText,
  FolderOpen,
  FolderTree,
  HardDriveDownload,
  LoaderCircle,
  MapPin,
  RefreshCw,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  X,
} from "lucide-react"
import {
  bridgeError,
  bridgeFailureKind,
  controlManagedRoot,
  deleteSourceRoot,
  exportDiagnostics,
  importSelectedRoot,
  readDiagnostics,
  revealSourceFile,
  requestSearchCancel,
  rescanManagedRoot,
  searchDeadlineMs,
  searchOutcome,
  searchResumes,
  selectImportRoot,
  type DiagnosticsBody,
  type SourceRoot,
  type SearchHit,
} from "./daemon"
import { useDetailSession } from "./detail-session"
import { DetailDrawer } from "./detail-drawer"
import {
  daemonRetryControl,
  deletionReceiptUncertainPresentation,
  rootsAfterDeletionAccepted,
  sourcePanelBanner,
  useDaemonRuntime,
} from "./daemon-runtime"
import { DiagnosticsContent, type DiagnosticsState } from "./diagnostics-panel"
import {
  CapabilityMatrix,
  IndexServiceSummary,
  indexServicePresentation,
  lifecycleLabel,
} from "./daemon-health"
import { SourceRootsPanel } from "./source-roots-panel"

export { IndexServiceSummary, indexServicePresentation } from "./daemon-health"

type ViewState = "idle" | "loading" | "complete" | "partial" | "empty" | "overload" | "cancelled" | "error"
type Mode = "keyword" | "field" | "hybrid" | "semantic"
type Degree = "" | "associate" | "bachelor" | "master" | "doctorate"
type Overlay = "import" | "diagnostics" | null

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

export function App() {
  const previewMode = import.meta.env.DEV ? new URLSearchParams(window.location.search).get("preview") : null
  const preview = previewMode === "search" || previewMode === "detail" || previewMode === "import"
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
  const [diagnosticsState, setDiagnosticsState] = useState<DiagnosticsState>("idle")
  const [diagnosticsMessage, setDiagnosticsMessage] = useState("尚未读取本地脱敏诊断")
  const [diagnostics, setDiagnostics] = useState<DiagnosticsBody | null>(null)
  const [pendingDeletions, setPendingDeletions] = useState<Record<string, {
    displayLabel: string
    affectedDocuments: number
  }>>({})
  const cancelToken = useRef<string | null>(null)
  const previewDetailOpened = useRef(false)
  const {
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
  } = useDaemonRuntime({
    preview,
    previewImport: previewMode === "import",
    sourcePanelOpen: overlay === "import",
  })
  useEffect(() => {
    const activeRootIds = new Set(managedRoots.map((root) => root.root_id))
    const completed = Object.entries(pendingDeletions).filter(
      ([rootId]) => !activeRootIds.has(rootId),
    )
    if (completed.length === 0) return
    if (selectedRoot && completed.some(([rootId]) => rootId === selectedRoot.root_id)) {
      setSelectedRoot(null)
    }
    setImportState("selected")
    const affectedDocuments = completed.reduce(
      (total, [, deletion]) => total + deletion.affectedDocuments,
      0,
    )
    const labels = completed.map(([, deletion]) => `“${deletion.displayLabel}”`).join("、")
    setImportMessage(`已删除${labels}的 ${affectedDocuments.toLocaleString()} 份本地派生数据；源文件未改动`)
    setPendingDeletions((current) => Object.fromEntries(
      Object.entries(current).filter(([rootId]) => activeRootIds.has(rootId)),
    ))
  }, [managedRoots, pendingDeletions, selectedRoot, setImportMessage, setImportState, setSelectedRoot])
  const {
    detail,
    detailLoading,
    detailError,
    fullText,
    bodyComplete,
    detailInterrupted,
    selectedHit,
    open: openDetail,
    loadText: loadDetailText,
    resume: resumeDetail,
    reset: resetDetail,
    observeAuthority: observeDetailAuthority,
    observeLifecycle: observeDetailLifecycle,
  } = useDetailSession({
    preview,
    authorityRef: actionAuthorityRef,
    lifecycleRef,
    service,
    isCapabilityAuthorized: () => capabilityAuthorized("detail"),
    onStaleSelection: () => setResultFreshness("stale"),
  })
  useLayoutEffect(() => {
    bindDetailObservers({ observeAuthority: observeDetailAuthority, observeLifecycle: observeDetailLifecycle })
  }, [bindDetailObservers, observeDetailAuthority, observeDetailLifecycle])

  const terms = useMemo(() => queryTerms(query), [query])
  const filterCount = [skills, location, degree, years].filter((value) => value.trim()).length
  const resultPageCount = Math.max(1, Math.ceil(results.length / RESULT_PAGE_SIZE))
  const visibleResults = results.slice(resultPage * RESULT_PAGE_SIZE, (resultPage + 1) * RESULT_PAGE_SIZE)
  const sourceTotals = useMemo(() => managedRoots.reduce((totals, root) => ({
    discovered: totals.discovered + root.current_counts.discovered,
    searchable: totals.searchable + root.current_counts.searchable,
    ocr: totals.ocr + root.current_counts.ocr,
    failed: totals.failed + root.current_counts.failed,
  }), { discovered: 0, searchable: 0, ocr: 0, failed: 0 }), [managedRoots])
  const searchablePercent = authoritativeStatus?.indexed_documents && authoritativeStatus.searchable_documents !== null ? Math.round((authoritativeStatus.searchable_documents / authoritativeStatus.indexed_documents) * 100) : 0
  const health = lifecycle.state === "running" && runtimeView === "trusted"
    ? service === "ready" ? "ok" : "degraded"
    : lifecycle.state === "starting" || lifecycle.state === "retry_wait" ? "loading" : "unavailable"
  const searchCapability = authoritativeStatus?.capabilities[mode === "semantic" ? "semantic_search" : mode === "hybrid" ? "hybrid_search" : "keyword_search"]
  const searchAllowed = runtimeView === "trusted" && lifecycle.state === "running" && searchCapability !== undefined && ["available", "degraded"].includes(searchCapability.state)
  const detailAllowed = runtimeView === "trusted" && lifecycle.state === "running" && authoritativeStatus?.capabilities.detail.state === "available"
  const importAllowed = runtimeView === "trusted" && lifecycle.state === "running" && authoritativeStatus?.capabilities.text_import.state === "available"
  const operationsPaused = !detailAllowed
  const retryControl = daemonRetryControl(lifecycle)
  const importBanner = sourcePanelBanner(importState, importMessage, managedRootsReadFailure)
  useEffect(() => {
    if (previewMode !== "detail" || previewDetailOpened.current) return
    previewDetailOpened.current = true
    void openDetail(PREVIEW_RESULTS[0])
  }, [previewMode])

  async function runSearch(event: FormEvent) {
    event.preventDefault()
    if (!query.trim() || view === "loading" || !searchAllowed) return
    if (preview) { setView("complete"); setResults(PREVIEW_RESULTS); setResultPage(0); setLatency(42); setResultFreshness("current"); setMessage("命中 5 条"); return }
    const capability = mode === "semantic" ? "semantic_search" : mode === "hybrid" ? "hybrid_search" : "keyword_search"
    const authority = captureCapabilityAuthority(capability, true)
    if (!authority) return
    const id = crypto.randomUUID()
    const startedGeneration = authority.generation
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
      if (!capabilityAuthorityIsCurrent(authority, capability, true)) {
        setResultFreshness("interrupted")
        setView(results.length > 0 ? previousView : "error")
        setMessage("daemon 已换代，本次搜索已中断；结果未自动重放")
        return
      }
      const body = reply.body
      const outcome = searchOutcome(reply)
      if (body.schema_version === "resume-ir.error.v3") {
        if (body.error.code === "REPAIRING" || body.error.code === "METADATA_UNAVAILABLE" || body.error.code === "QUERY_SERVICE_UNAVAILABLE") {
          setService(body.error.code === "REPAIRING" ? "repairing" : "degraded")
          setResultFreshness(results.length > 0 ? "interrupted" : "current")
          setView(results.length > 0 ? previousView : "error")
          setMessage(body.error.code === "REPAIRING" ? "索引正在修复；现有结果已保留" : "查询服务暂时不可用；现有结果已保留")
        } else if (body.error.code === "SERVICE_INITIALIZING" || body.error.code === "SERVICE_BLOCKED" || body.error.code === "CAPABILITY_UNAVAILABLE") {
          setView(results.length > 0 ? previousView : "error")
          setMessage(body.error.code === "SERVICE_INITIALIZING" ? "正在打开当前本地数据或恢复未完成操作；现有结果已保留" : "当前操作能力不可用；现有结果已保留")
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
      if (failure === "unavailable" || !capabilityAuthorityIsCurrent(authority, capability, true)) {
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
    const authority = captureActionAuthority()
    if (!authority) return
    try {
      await requestSearchCancel(`gui-cancel-command-${crypto.randomUUID()}`, token)
      if (!actionAuthorityIsCurrent(authority)) return
    } catch (error) {
      if (!actionAuthorityIsCurrent(authority)) return
      setMessage(bridgeFailureKind(error) === "overload" ? "取消入口繁忙，本次查询仍在执行" : "取消请求未送达，本次查询仍在执行")
    }
  }

  async function chooseImportRoot() {
    setImportState("selecting"); setImportMessage("正在打开本机目录选择器")
    try {
      const reply = await selectImportRoot()
      if (!reply) { setImportState("cancelled"); setImportMessage("未选择目录"); return }
      if (reply.body.schema_version === "resume-ir.error.v3") {
        if (reply.body.error.reason === "source_root_deleting") {
          await refreshManagedRoots(true)
          setImportState("active")
          setImportMessage("该目录仍在删除本地派生数据；完成后可重新添加")
        } else if (reply.body.error.code === "SERVICE_INITIALIZING") {
          setImportState("submitting")
          setImportMessage("正在打开当前本地数据或恢复未完成操作，目录尚未添加")
        } else {
          await refreshManagedRoots(true)
          setImportState("error")
          setImportMessage("daemon 未接受目录授权；已重新读取当前目录状态")
        }
        return
      }
      const selected = reply.body.root
      setSelectedRoot(selected)
      await refreshManagedRoots()
      setSelectedRoot(selected)
      setImportState("selected")
      setImportMessage("目录已授权；点击“开始扫描”后才会导入")
    }
    catch (error) { const overload = bridgeFailureKind(error) === "overload"; setSelectedRoot(null); setImportState(overload ? "overload" : "error"); setImportMessage(overload ? "目录选择入口繁忙，请稍后重试" : bridgeError(error).message) }
  }

  async function requestRootScan(root: SourceRoot) {
    if (root.state === "offline") {
      setImportState("error"); setImportMessage("目录当前不可读取，请恢复磁盘或权限"); return
    }
    if (preview) {
      setSelectedRoot(root); setImportState("queued"); setImportMessage(root.last_scan ? "已开始增量重新扫描" : "首次扫描已经开始"); return
    }
    const authority = captureCapabilityAuthority("text_import")
    if (!authority) return
    setSelectedRoot(root); setImportState("submitting"); setImportMessage(root.last_scan ? "正在提交增量重新扫描" : "正在提交首次扫描")
    try {
      const reply = root.last_scan ? await rescanManagedRoot(root.root_id) : await importSelectedRoot(root.root_id)
      if (!capabilityAuthorityIsCurrent(authority, "text_import")) return
      if (!("root" in reply.body)) {
        const deleting = reply.body.error.reason === "source_root_deleting"
        setImportState(reply.http_status === 409 ? "active" : "error")
        setImportMessage(deleting ? "该目录仍在删除本地派生数据；完成后可重新添加" : reply.http_status === 409 ? "该目录正在扫描，无需重复提交" : "daemon 未接受目录扫描任务")
        return
      }
      const updatedRoot = reply.body.root
      setManagedRoots((current) => current.map((candidate) => candidate.root_id === root.root_id ? updatedRoot : candidate))
      setSelectedRoot(updatedRoot)
      setImportState("queued")
      setImportMessage(root.last_scan ? "已开始增量重新扫描" : "首次扫描已经开始")
      await refreshStatus()
    } catch (error) {
      if (!capabilityAuthorityIsCurrent(authority, "text_import")) return
      const overload = bridgeFailureKind(error) === "overload"
      setImportState(overload ? "overload" : "error")
      setImportMessage(overload ? "目录扫描入口繁忙，请稍后重试" : bridgeError(error).message)
    }
  }

  async function changeRootControl(root: SourceRoot, action: "pause" | "resume") {
    const authority = captureActionAuthority()
    if (!authority) return
    if (action === "resume" && root.state === "offline") {
      setImportState("unavailable")
      setImportMessage("目录当前不可读取，恢复磁盘或权限后才能恢复监控")
      return
    }
    setSelectedRoot(root)
    setRootControls((current) => ({ ...current, [root.root_id]: "loading" }))
    if (preview) {
      const watcherState = action === "pause" ? "paused" : "active"
      setRootControls((current) => ({ ...current, [root.root_id]: watcherState }))
      setManagedRoots((current) => current.map((candidate) => candidate.root_id === root.root_id ? { ...candidate, watcher_state: watcherState } : candidate))
      setImportState("selected")
      setImportMessage(watcherState === "paused"
        ? "已暂停此目录的监听与周期扫描，仍可手动重新扫描"
        : root.last_scan
          ? "已恢复监控，并开始追赶目录变更"
          : "已恢复监控；首次导入仍需点击“开始扫描”")
      return
    }
    try {
      const reply = await controlManagedRoot(root.root_id, action)
      if (!actionAuthorityIsCurrent(authority)) return
      if (reply.body.schema_version === "resume-ir.error.v3") {
        const deleting = reply.body.error.reason === "source_root_deleting"
        setImportState(deleting ? "active" : "error")
        setImportMessage(deleting ? "该目录仍在删除本地派生数据；完成后可重新添加" : "daemon 未接受目录监控操作，可重试读取状态")
        return
      }
      const updated = reply.body.root
      setManagedRoots((current) => current.map((candidate) => candidate.root_id === updated.root_id ? updated : candidate))
      setSelectedRoot(updated)
      setRootControls((current) => ({ ...current, [root.root_id]: updated.watcher_state === "paused" ? "paused" : "active" }))
      setImportState("selected")
      setImportMessage(action === "pause"
        ? "已暂停此目录的监听与周期扫描，仍可手动重新扫描"
        : root.last_scan
          ? "已恢复监控，并开始追赶目录变更"
          : "已恢复监控；首次导入仍需点击“开始扫描”")
      await refreshStatus()
    } catch (error) {
      if (!actionAuthorityIsCurrent(authority)) return
      const state = bridgeFailureKind(error) === "overload" ? "overload" : "error"
      setRootControls((current) => ({ ...current, [root.root_id]: state }))
      setImportState(state)
      setImportMessage(state === "overload" ? "目录监控入口繁忙，请稍后重试" : bridgeError(error).message)
    }
  }

  async function removeSourceRoot(root: SourceRoot) {
    const authority = captureActionAuthority()
    if (!authority) return
    setSelectedRoot(root)
    setImportState("submitting")
    setImportMessage(`正在删除“${root.display_label}”的本地派生数据`)
    try {
      const reply = await deleteSourceRoot(root.root_id)
      if (!actionAuthorityIsCurrent(authority)) return
      if (!("affected_documents" in reply.body)) {
        setImportState("error")
        setImportMessage("daemon 未完成目录删除")
        return
      }
      const affectedDocuments = reply.body.affected_documents
      setManagedRoots((current) => rootsAfterDeletionAccepted(current, root.root_id))
      setPendingDeletions((current) => ({
        ...current,
        [root.root_id]: {
        displayLabel: root.display_label,
        affectedDocuments,
        },
      }))
      setImportState("active")
      setImportMessage(`正在删除“${root.display_label}”的本地派生数据；源文件不会改动`)
      await refreshManagedRoots()
      await refreshStatus()
    } catch {
      if (!actionAuthorityIsCurrent(authority)) return
      const uncertain = deletionReceiptUncertainPresentation()
      setImportState(uncertain.state)
      setImportMessage(uncertain.message)
      await refreshManagedRoots(true)
      await refreshStatus()
    }
  }

  async function openDiagnostics() {
    resetDetail(); setOverlay("diagnostics"); setDiagnosticsState("loading"); setDiagnosticsMessage("正在读取本地聚合诊断")
    try { const reply = await readDiagnostics(); if (reply.http_status !== 200 || reply.body.schema_version !== "resume-ir.diagnostics.v10" || reply.body.privacy_boundary !== "redacted_local_aggregate") { setDiagnostics(null); setDiagnosticsState("blocked"); setDiagnosticsMessage("诊断合同未满足脱敏导出边界"); return } setDiagnostics(reply.body); setDiagnosticsState("ready"); setDiagnosticsMessage("只读聚合诊断已就绪") }
    catch (error) { const overload = bridgeFailureKind(error) === "overload"; setDiagnostics(null); setDiagnosticsState(overload ? "overload" : "error"); setDiagnosticsMessage(overload ? "诊断读取入口繁忙，请稍后重试" : bridgeError(error).message) }
  }

  async function saveDiagnostics() {
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
        <IndexServiceSummary lifecycle={lifecycle} service={service} status={authoritativeStatus} searchablePercent={searchablePercent} connectionMessage={connectionMessage} runtimeView={runtimeView} />
        {runtimeView === "trusted" && retryControl && <button type="button" className="plain-button wide-button" onClick={() => void retryLifecycle()} disabled={retryControl.disabled}>{retryControl.label}</button>}
        <div className="local-only"><HardDriveDownload size={14} />完全本地运行 · 不上传</div>
      </div>
    </aside>

    <main className="main-shell">
      <header className="topbar"><div><span>resume-ir</span><ChevronRight size={14} /><strong>搜索</strong></div><Pill tone={health === "ok" ? "ok" : lifecycle.state === "blocked" || runtimeView === "bridge_error" ? "err" : "warn"}>{lifecycleLabel(lifecycle, service, runtimeView)}</Pill></header>
      <CapabilityMatrix lifecycle={lifecycle} status={authoritativeStatus} runtimeView={runtimeView} />
      <form className="search-head" onSubmit={runSearch}>
        <div className="query-box"><Search size={16} /><input id="query" value={query} onChange={(event) => setQuery(event.target.value)} maxLength={512} placeholder="输入关键词，空格分隔多个 Query（默认 AND 交集）" />{query && <button type="button" className="icon-button" aria-label="清空" onClick={() => setQuery("")}><X size={16} /></button>}</div>
        <div className="search-controls">
          <div className="term-chain">{terms.length > 1 && terms.map((term, index) => <span key={term}>{index > 0 && <b>AND</b>}<Tag tone="primary">{term}</Tag></span>)}</div>
          <div className="control-actions"><div className="segmented">{MODE_OPTIONS.map((option) => <button type="button" key={option.value} className={mode === option.value ? "selected" : ""} onClick={() => { setMode(option.value); if (option.value === "field") setShowFilters(true) }}>{option.label}</button>)}</div><button type="button" className={showFilters ? "filter-button active" : "filter-button"} onClick={() => setShowFilters((open) => !open)}><SlidersHorizontal size={14} />过滤{filterCount ? ` · ${filterCount}` : ""}</button><button className="primary-button" type="submit" disabled={!searchAllowed || !query.trim() || view === "loading"}>{view === "loading" ? <LoaderCircle className="spin" size={15} /> : <Search size={15} />}搜索</button>{view === "loading" && <button type="button" className="plain-button" onClick={() => void cancelSearch()}>取消</button>}</div>
        </div>
      </form>

      <div className="content-shell">
        <section className="results-pane" aria-live="polite">
          {view !== "idle" && <div className={`execution-bar execution-${view}`}><div>{view === "loading" ? <LoaderCircle className="spin" size={15} /> : view === "error" || view === "overload" ? <AlertTriangle size={15} /> : <Pill tone={view === "partial" ? "warn" : view === "complete" ? "ok" : "neutral"}>{view === "partial" ? "部分结果" : view === "complete" ? "搜索完成" : "搜索状态"}</Pill>}<span>{message}</span>{latency !== null && <span><Clock3 size={14} />{latency.toFixed(0)} ms</span>}<span>已索引 {countLabel(authoritativeStatus?.searchable_documents)} / {countLabel(authoritativeStatus?.indexed_documents)}</span></div><div><Tag tone={mode === "semantic" || mode === "hybrid" ? "ok" : "neutral"}>语义</Tag><Tag>正文</Tag>{filterCount > 0 && <Tag tone="primary">字段 · {filterCount}</Tag>}</div></div>}
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

    {(detail || detailLoading || detailError) && <DetailDrawer hit={selectedHit} detail={detail} loading={detailLoading} error={detailError} interrupted={detailInterrupted} detailAllowed={detailAllowed} fullText={fullText} bodyComplete={bodyComplete} previewMode={preview} terms={terms} onClose={resetDetail} onLoadText={() => void loadDetailText()} onResume={() => void resumeDetail()} onReveal={async () => {
      if (!selectedHit) throw new Error("selection unavailable")
      await revealSourceFile(selectedHit.selection)
    }} />}

    {overlay === "import" && <SlideOver title="简历来源" subtitle="本地目录只由原生进程持有" onClose={() => setOverlay(null)}>
      <div className="sheet-scroll import-content">
        <div className={`banner banner-${["error", "mismatch", "overload"].includes(importBanner.state) ? "err" : ["queued", "pending", "active"].includes(importBanner.state) ? "ok" : "neutral"}`} aria-live="polite">
          {["selecting", "submitting"].includes(importBanner.state) ? <LoaderCircle className="spin" size={16} /> : ["error", "mismatch", "overload", "unavailable"].includes(importBanner.state) ? <AlertTriangle size={16} /> : <FolderOpen size={16} />}
          <span>{importBanner.message}</span>
        </div>
        <SourceRootsPanel
          roots={managedRoots}
          busy={["selecting", "submitting"].includes(importState)}
          importAllowed={importAllowed}
          onAdd={() => void chooseImportRoot()}
          onScan={(root) => void requestRootScan(root)}
          onPause={(root) => void changeRootControl(root, "pause")}
          onResume={(root) => void changeRootControl(root, "resume")}
          onDelete={(root) => void removeSourceRoot(root)}
        />
        <section className="panel-card source-summary"><header><strong>当前本地索引</strong></header><dl><div><dt>已发现</dt><dd>{managedRoots.length > 0 ? sourceTotals.discovered.toLocaleString() : "—"}</dd></div><div><dt>可搜索</dt><dd>{authoritativeStatus?.searchable_documents ?? (managedRoots.length > 0 ? sourceTotals.searchable.toLocaleString() : "—")}</dd></div><div><dt>OCR 待处理</dt><dd>{authoritativeStatus?.ocr_queue_depth ?? (managedRoots.length > 0 ? sourceTotals.ocr.toLocaleString() : "—")}</dd></div><div><dt>失败</dt><dd>{managedRoots.length > 0 ? sourceTotals.failed.toLocaleString() : "—"}</dd></div></dl></section>
      </div>
    </SlideOver>}
    {overlay === "diagnostics" && <SlideOver title="隐私与诊断" subtitle="敏感详情可在本地展示；导出证据仍保持脱敏" onClose={() => setOverlay(null)}><DiagnosticsContent state={diagnosticsState} message={diagnosticsMessage} diagnostics={diagnostics} onExport={() => void saveDiagnostics()} /></SlideOver>}
  </div>
}
