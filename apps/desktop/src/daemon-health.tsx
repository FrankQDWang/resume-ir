import type { CapabilityReason, CoreReason, OptionalRuntimeReason, StatusBody } from "./daemon"
import {
  blockedReasonMessage,
  type DaemonExitClass,
  type DaemonLifecycleSnapshot,
  type DaemonService,
  type DaemonTransitionReason,
} from "./runtime-state"

export type RuntimeView = "trusted" | "bridge_error" | "service_unknown"

export function lifecycleMessage(snapshot: DaemonLifecycleSnapshot): string {
  if (snapshot.state === "starting") return "正在启动本地 daemon"
  if (snapshot.state === "retry_wait") return snapshot.retry_after_ms === null
    ? "daemon 正在恢复"
    : `daemon 正在恢复，约 ${snapshot.retry_after_ms}ms 后重试`
  if (snapshot.state === "circuit_open") return "daemon 连续异常，恢复电路暂时开路"
  if (snapshot.state === "blocked") return blockedReasonMessage(snapshot.transition_reason)
  return "daemon 控制面已就绪"
}

export function lifecycleLabel(
  snapshot: DaemonLifecycleSnapshot,
  service: DaemonService,
  runtimeView: RuntimeView = "trusted",
): string {
  if (runtimeView === "bridge_error") return "生命周期不可读"
  if (runtimeView === "service_unknown") return "服务状态未知"
  if (snapshot.state === "running") {
    if (service === "ready") return "daemon 可用"
    if (service === "initializing") return "daemon 初始化中"
    if (service === "repairing") return "daemon 修复中"
    if (service === "blocked") return "daemon 服务已阻塞"
    return "daemon 服务降级"
  }
  if (snapshot.state === "starting") return "daemon 启动中"
  if (snapshot.state === "retry_wait") return "daemon 恢复中"
  if (snapshot.state === "circuit_open") return "daemon 已开路"
  return "daemon 已阻止"
}

function countLabel(value: number | null | undefined): string {
  return value === null || value === undefined ? "—" : value.toLocaleString()
}

export function indexServicePresentation(
  service: DaemonService,
  coreReason: StatusBody["core"]["reason"],
): { title: string; message: string } {
  if (service === "ready") return { title: "索引可用", message: "daemon 可用" }
  if (service === "unknown") return { title: "服务状态未知", message: "状态读取失败，所有数据面操作已撤销" }
  if (service === "initializing") return { title: "服务初始化中", message: "daemon 控制面已就绪，正在打开本地 v29 数据" }
  if (service === "repairing") {
    if (coreReason === "migration_rebuild") return { title: "索引修复中", message: "正在重建当前索引" }
    return { title: "索引修复中", message: "daemon 已连接，索引正在修复" }
  }
  if (coreReason === "unsupported_store_schema") {
    return { title: "数据版本不受支持", message: "当前版本只接受 schema v29；原数据保持未修改" }
  }
  if (coreReason === "runtime_invariant") {
    return { title: "索引修复已阻塞", message: "daemon 已连接，请导出脱敏诊断" }
  }
  if (coreReason === "source_unavailable") {
    return { title: "来源不可用", message: "请恢复来源磁盘连接或文件权限" }
  }
  return { title: "索引能力降级", message: "daemon 已连接，部分能力不可用" }
}

export function IndexServiceSummary({
  lifecycle,
  service,
  status,
  searchablePercent,
  connectionMessage,
  runtimeView,
}: {
  lifecycle: DaemonLifecycleSnapshot
  service: DaemonService
  status: Pick<StatusBody, "core" | "searchable_documents" | "ocr_queue_depth"> | null
  searchablePercent: number
  connectionMessage: string
  runtimeView: RuntimeView
}) {
  const presentation = indexServicePresentation(service, status?.core.reason ?? null)
  const healthy = runtimeView === "trusted" && lifecycle.state === "running" && service === "ready"
  const title = lifecycle.state === "running"
    ? presentation.title
    : lifecycleLabel(lifecycle, service, runtimeView)
  const message = healthy && status
    ? `${countLabel(status.searchable_documents)} 份可搜索 · OCR 队列 ${countLabel(status.ocr_queue_depth)}`
    : lifecycle.state === "running" ? presentation.message : connectionMessage
  return <>
    <div className="status-title"><strong>{title}</strong><span className={healthy ? "ok-text" : "warn-text"}>{searchablePercent}%</span></div>
    <progress className="progress-track" data-health={healthy ? "ok" : "warn"} value={searchablePercent} max={100} aria-label="可搜索简历比例" />
    <p>{message}</p>
  </>
}

const runtimeLabels = { embedding: "语义运行时", ocr: "OCR 运行时", classifier: "分类器" } as const
const capabilityLabels = { keyword_search: "关键词检索", detail: "详情", semantic_search: "语义检索", hybrid_search: "混合检索", text_import: "文本导入", ocr_import: "OCR 导入", index_publication: "索引发布" } as const
const runtimeReasonLabels: Record<OptionalRuntimeReason, string> = { missing: "缺失", invalid: "完整性无效", start_failed: "启动失败", not_configured: "未配置" }
const capabilityReasonLabels: Record<CapabilityReason, string> = { core_initializing: "核心初始化中", core_blocked: "核心已阻塞", embedding_unavailable: "语义运行时不可用", ocr_unavailable: "OCR 运行时不可用", classifier_unavailable: "分类器不可用" }
const coreReasonLabels: Record<CoreReason, string> = { metadata_initializing: "元数据初始化中", migration_rebuild: "索引重建中", artifact_unavailable: "索引产物不可用", source_unavailable: "来源不可用", runtime_invariant: "运行时不变量失败", unsupported_store_schema: "存储 schema 不受支持", metadata_unavailable: "元数据不可用" }

const transitionReasonLabels: Record<DaemonTransitionReason, string> = {
  initial_start: "首次启动",
  automatic_retry: "自动重试",
  manual_retry: "人工重试",
  control_plane_ready: "控制面就绪",
  child_exited: "子进程退出",
  startup_timeout: "启动超时",
  heartbeat_timeout: "心跳超时",
  start_failed: "启动失败",
  control_plane_failure: "控制面故障",
  restart_budget_exhausted: "重启预算耗尽",
  half_open_retry: "半开重试",
  configuration_invalid: "配置无效",
  runtime_integrity: "运行时完整性失败",
  protocol_mismatch: "协议不匹配",
  ownership_conflict: "数据目录所有权冲突",
  supervisor_unavailable: "监督器不可用",
}
const exitLabels: Record<DaemonExitClass, string> = {
  child_exited: "子进程退出",
  startup_timeout: "启动超时",
  heartbeat_timeout: "心跳超时",
  start_failed: "启动失败",
  control_plane_failure: "控制面故障",
}

export function CapabilityMatrix({ lifecycle, status, runtimeView }: { lifecycle: DaemonLifecycleSnapshot; status: StatusBody | null; runtimeView: RuntimeView }) {
  const processSummary = `进程 ${lifecycle.state} · generation ${lifecycle.generation}`
  const reason = transitionReasonLabels[lifecycle.transition_reason]
  const lastExit = lifecycle.last_exit === null ? "无" : exitLabels[lifecycle.last_exit]
  if (runtimeView !== "trusted" || status === null) {
    return <section className="panel-card" aria-label="daemon 能力状态">
      <header><strong>{runtimeView === "bridge_error" ? "生命周期不可读" : "服务状态未知"}</strong><span>{processSummary}</span></header>
      <div className="tag-row"><span className="tag tag-warn">状态原因 · {reason}</span><span className="tag tag-neutral">上次退出 · {lastExit}</span></div>
      <div className="state-banner"><span>旧健康快照不会继续授权任何操作。</span></div>
    </section>
  }
  return <section className="panel-card" aria-label="daemon 能力状态">
    <header><strong>进程与能力</strong><span>{processSummary} · core {status.core.state}</span></header>
    <div className="tag-row"><span className="tag tag-neutral">状态原因 · {reason}</span><span className="tag tag-neutral">上次退出 · {lastExit}</span>{status.core.reason !== null && <span className="tag tag-warn">core 原因 · {coreReasonLabels[status.core.reason]}</span>}</div>
    <div className="tag-row">
      {Object.entries(status.optional_runtimes).map(([name, value]) => <span className={`tag tag-${value.state === "available" ? "ok" : value.state === "unavailable" ? "warn" : "neutral"}`} key={name}>{runtimeLabels[name as keyof typeof runtimeLabels]} · {value.state}{value.reason === null ? "" : ` · ${runtimeReasonLabels[value.reason]}`}</span>)}
    </div>
    <div className="tag-row">
      {Object.entries(status.capabilities).map(([name, value]) => <span className={`tag tag-${value.state === "available" ? "ok" : value.state === "degraded" || value.state === "unavailable" || value.state === "blocked" ? "warn" : "neutral"}`} key={name}>{capabilityLabels[name as keyof typeof capabilityLabels]} · {value.state}{value.reason === null ? "" : ` · ${capabilityReasonLabels[value.reason]}`}</span>)}
    </div>
  </section>
}
