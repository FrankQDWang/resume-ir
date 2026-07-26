import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react"
import {
  AlertTriangle,
  FolderSearch,
  LoaderCircle,
  RotateCcw,
  X,
} from "lucide-react"

import type { DetailViewDocument } from "./detail-session"
import type { SearchHit } from "./daemon"
import { PdfPreview } from "./pdf-preview"

const DEFAULT_WIDTH = 760
const MIN_WIDTH = 480
const MAX_WIDTH = 1180
const WIDTH_STORAGE_KEY = "resume-ir.detail-drawer-width.v1"
const systemLocationLabel = navigator.userAgent.includes("Windows")
  ? "在文件资源管理器中显示"
  : "在访达中显示"
const systemLocationSuccess = navigator.userAgent.includes("Windows")
  ? "已在文件资源管理器中选中来源文件"
  : "已在访达中选中来源文件"

type DetailTab = "original" | "fields" | "text"

function extension(fileName: string): string {
  return fileName.split(".").pop()?.toUpperCase() ?? "FILE"
}

function stem(fileName: string): string {
  return fileName.replace(/\.[^.]+$/, "").replaceAll("_", " ")
}

function initialWidth(): number {
  const stored = Number.parseInt(localStorage.getItem(WIDTH_STORAGE_KEY) ?? "", 10)
  return Number.isFinite(stored) ? Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, stored)) : DEFAULT_WIDTH
}

function boundedWidth(value: number): number {
  return Math.min(Math.min(MAX_WIDTH, window.innerWidth - 48), Math.max(MIN_WIDTH, value))
}

export function DetailDrawer({
  hit,
  detail,
  loading,
  error,
  interrupted,
  detailAllowed,
  fullText,
  bodyComplete,
  previewMode,
  terms,
  onClose,
  onLoadText,
  onResume,
  onReveal,
}: {
  hit: SearchHit | null
  detail: DetailViewDocument | null
  loading: boolean
  error: string
  interrupted: boolean
  detailAllowed: boolean
  fullText: string
  bodyComplete: boolean
  previewMode: boolean
  terms: string[]
  onClose: () => void
  onLoadText: () => void
  onResume: () => void
  onReveal: () => Promise<void>
}) {
  const fileName = detail?.file_name ?? hit?.file_name ?? "简历详情"
  const isPdf = extension(fileName) === "PDF"
  const [width, setWidth] = useState(initialWidth)
  const [tab, setTab] = useState<DetailTab>(() => isPdf ? "original" : "fields")
  const [revealBusy, setRevealBusy] = useState(false)
  const [revealMessage, setRevealMessage] = useState("")
  const dragStart = useRef<{ x: number; width: number; current: number } | null>(null)

  useEffect(() => {
    setTab(isPdf ? "original" : "fields")
    setRevealMessage("")
  }, [hit?.selection.doc_id, hit?.selection.version_id, isPdf])

  useEffect(() => {
    const resize = () => setWidth((current) => boundedWidth(current))
    window.addEventListener("resize", resize)
    return () => window.removeEventListener("resize", resize)
  }, [])

  const commitWidth = (next: number) => {
    const bounded = boundedWidth(next)
    setWidth(bounded)
    localStorage.setItem(WIDTH_STORAGE_KEY, String(Math.round(bounded)))
  }

  const beginResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    dragStart.current = { x: event.clientX, width, current: width }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const resize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!dragStart.current) return
    const next = boundedWidth(dragStart.current.width + dragStart.current.x - event.clientX)
    dragStart.current.current = next
    setWidth(next)
  }

  const finishResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!dragStart.current) return
    const next = dragStart.current.current
    dragStart.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    setWidth(next)
    localStorage.setItem(WIDTH_STORAGE_KEY, String(Math.round(next)))
  }

  const resetWidth = () => {
    localStorage.removeItem(WIDTH_STORAGE_KEY)
    setWidth(boundedWidth(DEFAULT_WIDTH))
  }

  return <div className="overlay detail-overlay" role="dialog" aria-modal="true" aria-label={stem(fileName)}>
    <button type="button" className="overlay-backdrop" aria-label="关闭" onClick={onClose} />
    <section className="sheet detail-sheet" style={{ width, maxWidth: "calc(100vw - 48px)" }}>
      <div
        className="sheet-resize-handle"
        role="separator"
        aria-label="调整详情宽度"
        aria-orientation="vertical"
        tabIndex={0}
        onPointerDown={beginResize}
        onPointerMove={resize}
        onPointerUp={finishResize}
        onPointerCancel={finishResize}
        onDoubleClick={resetWidth}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") commitWidth(width + 24)
          if (event.key === "ArrowRight") commitWidth(width - 24)
          if (event.key === "Home") resetWidth()
        }}
      />
      <header className="sheet-header detail-sheet-header">
        <div><h2>{stem(fileName)}</h2><p>{detail ? `${extension(fileName)} · ${Math.ceil(detail.source_byte_size / 1024)} KiB` : "正在读取本地详情"}</p></div>
        <div className="detail-header-actions">
          <button type="button" className="plain-button" onClick={() => {
            setRevealBusy(true)
            setRevealMessage("")
            void onReveal()
              .then(() => setRevealMessage(systemLocationSuccess))
              .catch(() => setRevealMessage("来源文件已移动、变化或暂时不可用"))
              .finally(() => setRevealBusy(false))
          }} disabled={!detail || previewMode || revealBusy}><FolderSearch size={14} />{revealBusy ? "正在定位…" : systemLocationLabel}</button>
          <button type="button" className="icon-button" onClick={resetWidth} aria-label="恢复默认宽度"><RotateCcw size={15} /></button>
          <button type="button" className="icon-button" onClick={onClose} aria-label="关闭面板"><X size={16} /></button>
        </div>
      </header>
      <nav className="detail-tabs" aria-label="简历详情视图">
        {isPdf && <button type="button" className={tab === "original" ? "active" : ""} onClick={() => setTab("original")}>原始简历</button>}
        <button type="button" className={tab === "fields" ? "active" : ""} onClick={() => setTab("fields")}>结构化字段（用于筛选）</button>
        <button type="button" className={tab === "text" ? "active" : ""} onClick={() => { setTab("text"); onLoadText() }}>提取文本</button>
      </nav>
      <div className="sheet-scroll detail-content">
        {revealMessage && <div className={`banner ${revealMessage.startsWith("已") ? "banner-ok" : "banner-err"}`}>{revealMessage}</div>}
        {loading && !detail && <div className="detail-loading"><LoaderCircle className="spin" size={20} />正在读取精确版本详情</div>}
        {error && <div className="banner banner-err"><AlertTriangle size={16} />{error}</div>}
        {interrupted && detailAllowed && <button type="button" className="plain-button wide-button" onClick={onResume} disabled={loading}>显式续读当前版本</button>}
        {detail && hit && tab === "original" && (previewMode
          ? <div className="pdf-preview-state">预览模式不读取本机源文件</div>
          : <PdfPreview selection={hit.selection} fileName={fileName} />)}
        {detail && tab === "fields" && <>
          <section className="detail-section first-detail-section"><h3>搜索命中摘要</h3><p className="snippet-box">{detail.snippet || "（无命中摘要）"}</p><div className="tag-row">{terms.map((term) => <span className="tag tag-primary" key={term}>命中：{term}</span>)}</div></section>
          <section className="detail-section"><h3>结构化字段</h3><p className="detail-explainer">这些带置信度的派生字段用于筛选和解释，不参与普通关键词或混合排序。</p><dl className="field-grid">{detail.fields.slice(0, 32).map((field, index) => <div key={`${field.type}-${index}`}><dt>{field.type}</dt><dd>{field.value}<small>{Math.round(field.confidence * 100)}%</small></dd></div>)}</dl>{detail.fields_truncated && <small className="muted-note">字段已按本地响应上限截断</small>}</section>
          <section className="file-panel"><div><span>文件类型</span><strong>{extension(fileName)}</strong></div><div><span>来源大小</span><strong>{Math.ceil(detail.source_byte_size / 1024)} KiB</strong></div><div><span>解析合同</span><code>{detail.parse_version} · {detail.schema_version}</code></div><div><span>语言 / 页数</span><strong>{detail.language_set.join("、") || "—"} · {detail.page_count ?? "—"}</strong></div></section>
        </>}
        {detail && tab === "text" && <section className="detail-section first-detail-section"><h3>提取文本</h3><pre className="full-text">{fullText || (loading ? "正在读取…" : "（正文为空）")}</pre>{!bodyComplete && fullText && <small className="muted-note">{interrupted ? "正文读取已中断；现有内容保持不变。" : loading ? "正在继续读取同一版本正文…" : "正文超过桌面展示上限。"}</small>}</section>}
      </div>
    </section>
  </div>
}
