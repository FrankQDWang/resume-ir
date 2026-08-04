import { FolderOpen, FolderTree, Pause, Play, RefreshCw, Trash2, X } from "lucide-react"
import { useState } from "react"

import type { CapabilityState, SourceRoot } from "./daemon"
import { ImportProgress } from "./import-progress"

interface SourceRootsPanelProps {
  roots: SourceRoot[]
  busy: boolean
  importAllowed: boolean
  keywordCapabilityState: CapabilityState
  onAdd(): void
  onScan(root: SourceRoot): void
  onPause(root: SourceRoot): void
  onResume(root: SourceRoot): void
  onDelete(root: SourceRoot): void
}

const ACTIVE_PHASES = new Set(["queued", "discovering", "fingerprinting", "classifying", "parsing", "ocr", "publishing"])

export function SourceRootsPanel({
  roots,
  busy,
  importAllowed,
  keywordCapabilityState,
  onAdd,
  onScan,
  onPause,
  onResume,
  onDelete,
}: SourceRootsPanelProps) {
  const [confirming, setConfirming] = useState<SourceRoot | null>(null)
  return <section className="panel-card source-roots-panel">
    <header>
      <div><strong>已授权目录</strong><span>{roots.length} / 16</span></div>
      <button type="button" className="plain-button" onClick={onAdd} disabled={busy || !importAllowed || roots.length >= 16}>
        <FolderOpen size={14} />添加目录
      </button>
    </header>
    {roots.length === 0
      ? <div className="source-empty"><FolderTree size={24} /><div><strong>尚未选择目录</strong><p>目录扫描、解析、分类与索引全部在本机完成。</p></div></div>
      : roots.map((root) => <SourceRootCard
        key={root.root_id}
        root={root}
        busy={busy}
        importAllowed={importAllowed}
        keywordCapabilityState={keywordCapabilityState}
        onScan={() => onScan(root)}
        onPause={() => onPause(root)}
        onResume={() => onResume(root)}
        onDelete={() => setConfirming(root)}
      />)}
    {confirming && <div className="source-delete-confirm" role="alertdialog" aria-modal="true" aria-label="确认删除目录数据">
      <div className="source-delete-copy">
        <strong>删除“{confirming.display_label}”及其本地数据？</strong>
        <p>
          将处理 {confirming.current_counts.discovered.toLocaleString()} 个已发现文件，
          从搜索中移除 {confirming.current_counts.searchable.toLocaleString()} 份简历，
          并清理 {confirming.current_counts.ocr.toLocaleString()} 个 OCR 待办和
          {confirming.current_counts.failed.toLocaleString()} 个失败记录对应的应用派生数据。
          不会删除磁盘上的源文件。
        </p>
      </div>
      <button type="button" className="icon-button" onClick={() => setConfirming(null)} aria-label="取消删除"><X size={15} /></button>
      <div className="source-delete-actions">
        <button type="button" className="plain-button" onClick={() => setConfirming(null)}>取消</button>
        <button type="button" className="danger-button" onClick={() => { const root = confirming; setConfirming(null); onDelete(root) }} disabled={busy}>
          <Trash2 size={14} />删除目录及本地数据
        </button>
      </div>
    </div>}
  </section>
}

function SourceRootCard({
  root,
  busy,
  importAllowed,
  keywordCapabilityState,
  onScan,
  onPause,
  onResume,
  onDelete,
}: {
  root: SourceRoot
  busy: boolean
  importAllowed: boolean
  keywordCapabilityState: CapabilityState
  onScan(): void
  onPause(): void
  onResume(): void
  onDelete(): void
}) {
  const scan = root.last_scan
  const deleting = root.state === "deleting"
  const active = scan !== null && ACTIVE_PHASES.has(scan.phase)
  const status = deleting
    ? "正在删除本地数据"
    : root.state === "offline"
    ? "目录离线"
    : root.watcher_state === "paused"
      ? "监控已暂停"
      : root.watcher_state === "unavailable"
        ? "监控不可用"
      : active
        ? "扫描进行中"
        : scan === null
          ? "等待首次扫描"
        : scan?.phase === "failed"
          ? "上次扫描失败"
          : scan?.phase === "partial"
            ? "上次扫描不完整"
            : "持续监控中"
  const scanLabel = active ? "扫描中" : scan ? "重新扫描" : "开始扫描"
  return <article className="source-root-card">
    <div className="source-root-heading">
      <FolderTree size={22} />
      <div className="source-copy"><strong>{root.display_label}</strong><p>{status} · {scan === null ? "首次扫描后启用自动监听与 5 分钟兜底" : "自动监听与每 5 分钟兜底扫描"}</p></div>
      <span className={`source-state source-state-${deleting ? "deleting" : root.watcher_state === "unavailable" ? "offline" : root.watcher_state === "paused" ? "paused" : root.state}`}>{status}</span>
    </div>
    <ImportProgress root={root} keywordCapabilityState={keywordCapabilityState} />
    <dl className="source-counts">
      <Count label="已发现" value={root.current_counts.discovered} />
      <Count label="可搜索" value={root.current_counts.searchable} />
      <Count label="非简历" value={root.current_counts.non_resume} />
      <Count label="待确认" value={root.current_counts.needs_review} />
      <Count label="OCR" value={root.current_counts.ocr} />
      <Count label="失败" value={root.current_counts.failed} />
      <Count label="忽略" value={scan?.counts.ignored} />
    </dl>
    <div className="source-actions">
      <button type="button" className="plain-button" onClick={onScan} disabled={busy || active || deleting || !importAllowed || root.state !== "active"}>
        <RefreshCw size={14} />{scanLabel}
      </button>
      {!deleting && root.state !== "offline" && root.watcher_state === "active"
        ? <button type="button" className="plain-button" onClick={onPause} disabled={busy}><Pause size={14} />暂停监控</button>
        : root.state !== "offline" && root.watcher_state === "paused"
          ? <button type="button" className="plain-button" onClick={onResume} disabled={busy}><Play size={14} />恢复监控</button>
          : null}
      <button type="button" className="plain-button danger-text" onClick={onDelete} disabled={busy || deleting}>
        <Trash2 size={14} />{deleting ? "删除中" : "删除"}
      </button>
    </div>
  </article>
}

function Count({ label, value }: { label: string; value: number | undefined }) {
  return <div><dt>{label}</dt><dd>{value === undefined ? "—" : value.toLocaleString()}</dd></div>
}
