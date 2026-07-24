import { AlertTriangle, CheckCircle2, Download, LoaderCircle, ShieldCheck } from "lucide-react"

import type { DiagnosticsBody } from "./daemon"

export type DiagnosticsState = "idle" | "loading" | "ready" | "exporting" | "saved" | "cancelled" | "blocked" | "overload" | "error"

function countLabel(value: number | null | undefined): string {
  return value === null || value === undefined ? "—" : value.toLocaleString()
}

export function DiagnosticsContent({ state, message, diagnostics, onExport }: { state: DiagnosticsState; message: string; diagnostics: DiagnosticsBody | null; onExport: () => void }) {
  const privacySafe = diagnostics === null || (!diagnostics.contains_raw_resume_text && !diagnostics.contains_queries && !diagnostics.contains_resume_paths && !diagnostics.contains_candidate_results && !diagnostics.contains_snippet_text)
  const unsafe = state === "error" || state === "blocked" || state === "overload"
  return <div className="sheet-scroll diagnostics-content">
    <div className={`banner banner-${unsafe ? "err" : state === "saved" ? "ok" : "neutral"}`} aria-live="polite">
      {state === "loading" || state === "exporting" ? <LoaderCircle className="spin" size={16} /> : unsafe ? <AlertTriangle size={16} /> : <ShieldCheck size={16} />}<span>{message}</span>
    </div>
    {diagnostics && <>
      <section className="panel-card"><header><strong>脱敏导出边界</strong><span className={`pill pill-${privacySafe ? "ok" : "err"}`}><span className="pill-dot" />{privacySafe ? "5/5 通过" : "阻止导出"}</span></header>
        {["简历正文", "查询文本", "原始路径", "候选结果", "结果摘要"].map((label) => <div className="check-row" key={label}><CheckCircle2 size={14} /><span>{label}</span><small>不包含</small></div>)}
      </section>
      <section className="panel-card"><header><strong>本地聚合</strong><span>{diagnostics.evidence_lane} · {diagnostics.evidence_status} · epoch {countLabel(diagnostics.visible_epoch)}</span></header><dl>
        <div><dt>已索引 / 可搜索</dt><dd>{countLabel(diagnostics.metrics.indexed_documents)} / {countLabel(diagnostics.metrics.searchable_documents)}</dd></div>
        <div><dt>OCR / embedding</dt><dd>{countLabel(diagnostics.metrics.ocr_queue_depth)} / {countLabel(diagnostics.metrics.embedding_queue_depth)}</dd></div>
        <div><dt>可恢复 / 永久失败</dt><dd>{countLabel(diagnostics.error_counts.failed_retryable)} / {countLabel(diagnostics.error_counts.failed_permanent)}</dd></div>
        <div><dt>查询 P95</dt><dd>{diagnostics.metrics.query_latency?.p95_ms === null || diagnostics.metrics.query_latency === null ? "—" : `${diagnostics.metrics.query_latency.p95_ms}ms`}</dd></div>
      </dl></section>
    </>}
    <button className="primary-button wide-button" onClick={onExport} disabled={!privacySafe || state === "exporting"}>{state === "exporting" ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}{diagnostics ? "导出脱敏 JSON" : "导出桌面生命周期诊断"}</button>
  </div>
}
