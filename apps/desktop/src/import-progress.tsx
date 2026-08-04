import { useEffect, useRef, useState } from "react"

import type { CapabilityState, SourceRoot } from "./daemon"

const ACTIVE_PHASES = new Set([
  "queued",
  "discovering",
  "fingerprinting",
  "classifying",
  "parsing",
  "ocr",
  "publishing",
])

export interface ImportProgressPresentation {
  active: boolean
  observedPercent: number
  exactCountLabel: string
  progressValueLabel: string
  stageMessage: string
  terminal: boolean
}

export function importProgressPresentation(
  root: SourceRoot,
  keywordCapabilityState: CapabilityState,
): ImportProgressPresentation {
  const scan = root.last_scan
  const active = scan !== null && ACTIVE_PHASES.has(scan.phase)
  const observedTotal = scan?.counts.total ?? scan?.counts.discovered ?? 0
  const processed = scan?.counts.processed ?? 0
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
  const fullyKeywordReady = scan?.phase === "complete"
    && scan.completeness === "complete"
    && allObservedFilesProcessed
    && root.current_counts.searchable > 0
    && root.current_counts.ocr === 0
    && keywordCapabilityState === "available"

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
    stageMessage: stageMessage(root, fullyKeywordReady),
    terminal,
  }
}

export function ImportProgress({
  root,
  keywordCapabilityState,
}: {
  root: SourceRoot
  keywordCapabilityState: CapabilityState
}) {
  const presentation = importProgressPresentation(root, keywordCapabilityState)
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

function stageMessage(root: SourceRoot, fullyKeywordReady: boolean): string {
  if (root.state === "deleting") return "正在删除本地派生数据"
  if (root.state === "offline") return "目录离线，等待恢复访问"
  if (root.watcher_state === "paused") return "目录监控已暂停"
  if (root.watcher_state === "unavailable") return "目录监控当前不可用"
  const scan = root.last_scan
  if (scan === null) return "等待首次扫描"
  if (scan.phase === "complete") {
    if (fullyKeywordReady) return "关键词检索全部可用"
    return root.current_counts.ocr > 0
      ? "本轮扫描完成，OCR 正在后台继续"
      : "本轮扫描完成"
  }
  if (scan.phase === "partial") return "本轮扫描不完整"
  if (scan.phase === "failed") return "本轮扫描失败"
  return ({
    queued: "等待扫描",
    discovering: "正在发现文件",
    fingerprinting: "正在核对文件变更",
    classifying: "正在识别简历与其他文件",
    parsing: "正在解析和处理",
    ocr: "正在处理 OCR 文件",
    publishing: "正在发布关键词和语义索引",
  } as Record<string, string>)[scan.phase] ?? "扫描进行中"
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
