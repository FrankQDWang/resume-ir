import { useEffect, useRef, useState } from "react"

import type { CapabilityState, SourceRoot, SourceRootScan } from "./daemon"

const ACTIVE_STAGE_LABELS: Partial<Record<SourceRootScan["phase"], string>> = {
  queued: "等待扫描",
  discovering: "发现文件",
  fingerprinting: "核对变更",
  classifying: "分类中",
  parsing: "解析中",
  ocr: "OCR 处理中",
  publishing: "发布索引",
}

/** Frontend-only: counters often pause during atomic full-text/vector publication. */
export const IMPORT_PROGRESS_STALL_MS = 8_000

export interface ImportProgressSignals {
  keywordCapabilityState: CapabilityState
  semanticCapabilityState?: CapabilityState
  embeddingQueueDepth?: number | null
  /** Local UI hint when processed/searchable stop advancing during an active scan. */
  indexPublishingHint?: boolean
}

export interface ImportProgressPresentation {
  active: boolean
  observedPercent: number
  exactCountLabel: string
  progressValueLabel: string
  stageMessage: string
  scanActionLabel: string
  terminal: boolean
  processed: number
  searchable: number
}

export function hasActiveManagedRootScan(roots: SourceRoot[]): boolean {
  return roots.some((root) => root.last_scan !== null && ACTIVE_STAGE_LABELS[root.last_scan.phase] !== undefined)
}

/** Short lifecycle/phase chip for the card header — not the progress-bar sentence. */
export function rootCardHeadingStatus(root: SourceRoot): string {
  if (root.state === "deleting") return "正在删除本地数据"
  if (root.state === "offline") return "目录离线"
  if (root.watcher_state === "paused") return "监控已暂停"
  if (root.watcher_state === "unavailable") return "监控不可用"
  const scan = root.last_scan
  if (scan === null) return "等待首次扫描"
  if (ACTIVE_STAGE_LABELS[scan.phase] !== undefined) {
    return ACTIVE_STAGE_LABELS[scan.phase] ?? "扫描中"
  }
  if (scan.phase === "failed") return "上次扫描失败"
  if (scan.phase === "partial") return "上次扫描不完整"
  return "持续监控中"
}

export function importProgressPresentation(
  root: SourceRoot,
  signals: ImportProgressSignals | CapabilityState,
): ImportProgressPresentation {
  const resolved = resolveSignals(signals)
  const scan = root.last_scan
  const active = scan !== null && ACTIVE_STAGE_LABELS[scan.phase] !== undefined
  const observedTotal = scan?.counts.total ?? scan?.counts.discovered ?? 0
  const processed = scan?.counts.processed ?? 0
  const searchable = root.current_counts.searchable
  const observedPercent = scan === null
    ? 0
    : observedTotal > 0
      ? Math.min(100, Math.round(processed / observedTotal * 100))
      : scan.phase === "complete"
        ? 100
        : 0
  const terminal = scan !== null && ["complete", "partial", "failed"].includes(scan.phase)
  const allObservedFilesProcessed = scan?.counts.total !== null
    && scan?.counts.total !== undefined
    && processed >= scan.counts.total
  const keywordCapabilityAvailable = resolved.keywordCapabilityState === "available"
  // OCR backlog must not block keyword-complete. Embedding still shares the
  // publication boundary until #418; queue depth is only a coarse global hint.
  const fullyKeywordReady = scan?.phase === "complete"
    && scan.completeness === "complete"
    && allObservedFilesProcessed
    && searchable > 0
    && keywordCapabilityAvailable
  const partiallyKeywordReady = searchable > 0 && keywordCapabilityAvailable
  const embeddingInProgress = (resolved.embeddingQueueDepth ?? 0) > 0
  const semanticAvailable = resolved.semanticCapabilityState === "available"

  const message = stageMessage(root, {
    active,
    observedPercent,
    searchable,
    fullyKeywordReady,
    partiallyKeywordReady,
    embeddingInProgress,
    semanticAvailable,
    indexPublishingHint: resolved.indexPublishingHint === true,
  })
  return {
    active,
    observedPercent,
    exactCountLabel: active
      ? `${processed.toLocaleString()} / ${observedTotal.toLocaleString()}`
      : lastSyncLabel(scan?.updated_at_seconds),
    progressValueLabel: active
      ? `${observedPercent}% · ${etaLabel(scan?.eta_seconds ?? null)}`
      : scan === null
        ? "未开始"
        : `${observedPercent}%`,
    stageMessage: message,
    scanActionLabel: active ? "扫描中" : scan ? "重新扫描" : "开始扫描",
    terminal,
    processed,
    searchable,
  }
}

export function ImportProgress({
  root,
  signals,
}: {
  root: SourceRoot
  signals: ImportProgressSignals
}) {
  const presentation = importProgressPresentation(root, signals)
  const scanId = root.last_scan?.scan_id ?? "not-started"
  const priorScanId = useRef(scanId)
  const [visualPercent, setVisualPercent] = useState(presentation.observedPercent)

  useEffect(() => {
    let firstFrame = 0
    let secondFrame = 0
    if (priorScanId.current !== scanId) {
      priorScanId.current = scanId
      setVisualPercent(0)
      firstFrame = window.requestAnimationFrame(() => {
        secondFrame = window.requestAnimationFrame(() => setVisualPercent(presentation.observedPercent))
      })
    } else {
      setVisualPercent(presentation.observedPercent)
    }
    return () => {
      window.cancelAnimationFrame(firstFrame)
      window.cancelAnimationFrame(secondFrame)
    }
  }, [presentation.observedPercent, scanId])

  return <div className="source-progress">
    <div className="source-progress-values">
      <span>{presentation.exactCountLabel}</span>
      <strong>{presentation.progressValueLabel}</strong>
    </div>
    <div
      className="source-progress-track"
      role="progressbar"
      aria-label={`${root.display_label} 扫描进度`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={presentation.observedPercent}
      aria-valuetext={`${presentation.observedPercent}%`}
    >
      <span
        className={`source-progress-fill${presentation.active ? " source-progress-fill-active" : ""}${presentation.terminal ? " source-progress-fill-terminal" : ""}`}
        style={{ transform: `scaleX(${visualPercent / 100})` }}
      />
    </div>
    <div className="source-progress-status">
      <span className="sr-only" role="status" aria-live="polite">
        {presentation.stageMessage}
      </span>
      <span
        key={presentation.stageMessage}
        className={`streaming-status-text${presentation.active ? " streaming-status-text-active" : ""}`}
        aria-hidden="true"
      >
        {presentation.stageMessage}
      </span>
    </div>
  </div>
}

export function useIndexPublishingHint(root: SourceRoot, signals: ImportProgressSignals): boolean {
  const baseline = importProgressPresentation(root, { ...signals, indexPublishingHint: false })
  const [stalled, setStalled] = useState(false)
  const marker = useRef({ scanId: root.last_scan?.scan_id ?? "", processed: baseline.processed, searchable: baseline.searchable })

  useEffect(() => {
    const scanId = root.last_scan?.scan_id ?? ""
    if (!baseline.active) {
      marker.current = { scanId, processed: baseline.processed, searchable: baseline.searchable }
      setStalled(false)
      return
    }
    if (
      marker.current.scanId !== scanId
      || marker.current.processed !== baseline.processed
      || marker.current.searchable !== baseline.searchable
    ) {
      marker.current = { scanId, processed: baseline.processed, searchable: baseline.searchable }
      setStalled(false)
    }
    const timer = window.setTimeout(() => {
      if (
        marker.current.scanId === scanId
        && marker.current.processed === baseline.processed
        && marker.current.searchable === baseline.searchable
      ) {
        setStalled(true)
      }
    }, IMPORT_PROGRESS_STALL_MS)
    return () => window.clearTimeout(timer)
  }, [baseline.active, baseline.processed, baseline.searchable, root.last_scan?.scan_id])

  return stalled
}

function resolveSignals(signals: ImportProgressSignals | CapabilityState): ImportProgressSignals {
  if (typeof signals === "string") {
    return { keywordCapabilityState: signals }
  }
  return signals
}

function stageMessage(
  root: SourceRoot,
  flags: {
    active: boolean
    observedPercent: number
    searchable: number
    fullyKeywordReady: boolean
    partiallyKeywordReady: boolean
    embeddingInProgress: boolean
    semanticAvailable: boolean
    indexPublishingHint: boolean
  },
): string {
  if (root.state === "deleting") return "正在删除本地派生数据"
  if (root.state === "offline") return "目录离线，等待恢复访问"
  if (root.watcher_state === "paused") return "目录监控已暂停"
  if (root.watcher_state === "unavailable") return "目录监控当前不可用"
  const scan = root.last_scan
  if (scan === null) return "等待首次扫描"
  if (scan.phase === "complete") {
    if (flags.fullyKeywordReady) {
      const parts = ["关键词检索全部可用"]
      if (root.current_counts.ocr > 0) parts.push("OCR 后台继续")
      if (flags.embeddingInProgress) parts.push("语义索引生成中")
      else if (flags.semanticAvailable) parts.push("语义检索可用")
      return parts.join(" · ")
    }
    return root.current_counts.ocr > 0
      ? "本轮扫描完成，OCR 正在后台继续"
      : "本轮扫描完成"
  }
  if (scan.phase === "partial") return "本轮扫描不完整"
  if (scan.phase === "failed") return "本轮扫描失败"

  const backendStage = ACTIVE_STAGE_LABELS[scan.phase] ?? "扫描进行中"
  const stage = flags.indexPublishingHint && scan.phase === "parsing"
    ? `索引发布中 ${flags.observedPercent}%`
    : scan.phase === "publishing"
      ? `发布索引 ${flags.observedPercent}%`
      : flags.active
        ? `${backendStage} ${flags.observedPercent}%`
        : backendStage
  const parts = [stage]
  if (flags.partiallyKeywordReady) {
    parts.push(`关键词检索已部分可用（已可搜 ${flags.searchable.toLocaleString()}）`)
  }
  if (flags.embeddingInProgress) parts.push("语义索引生成中")
  return parts.join(" · ")
}

function etaLabel(seconds: number | null) {
  if (seconds === null) return "估算中"
  if (seconds < 60) return `约 ${Math.max(1, seconds)} 秒`
  if (seconds < 3600) return `约 ${Math.ceil(seconds / 60)} 分钟`
  return `约 ${Math.ceil(seconds / 3600)} 小时`
}

function lastSyncLabel(timestamp: number | undefined) {
  if (timestamp === undefined) return "尚未扫描"
  return `上次同步 ${new Date(timestamp * 1000).toLocaleString()}`
}
