import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it } from "vitest"

import { CapabilityMatrix, IndexServiceSummary, indexServicePresentation } from "./daemon-health"
import type { StatusBody } from "./daemon"

const running = {
  schema_version: "resume-ir.desktop-daemon-lifecycle.v2" as const,
  state: "running" as const,
  transition_reason: "control_plane_ready" as const,
  generation: 3,
  automatic_restart_attempt: 0,
  automatic_restart_limit: 5 as const,
  retry_after_ms: null,
  heartbeat_failures: 0,
  last_exit: null,
}

describe("visible daemon health", () => {
  it("distinguishes unsupported v29 data, runtime invariant, and source failure", () => {
    expect(indexServicePresentation("blocked", "unsupported_store_schema")).toEqual({
      title: "数据版本不受支持",
      message: "当前版本只接受 schema v29；原数据保持未修改",
    })
    expect(indexServicePresentation("blocked", "runtime_invariant").message).toContain("导出脱敏诊断")
    expect(indexServicePresentation("degraded", "source_unavailable").message).toContain("来源磁盘")
  })

  it("shows service_unknown instead of retaining a stale ready presentation", () => {
    const markup = renderToStaticMarkup(createElement(IndexServiceSummary, {
      lifecycle: running,
      service: "unknown",
      status: null,
      searchablePercent: 0,
      connectionMessage: "状态读取失败",
      runtimeView: "service_unknown",
    }))
    expect(markup).toContain("服务状态未知")
    expect(markup).not.toContain("索引可用")
  })

  it("renders optional runtime and operation capability states independently", () => {
    const status = {
      core: { state: "ready", reason: null },
      optional_runtimes: {
        embedding: { state: "unavailable", reason: "invalid" },
        ocr: { state: "available", reason: null },
        classifier: { state: "available", reason: null },
      },
      capabilities: {
        keyword_search: { state: "available", reason: null },
        detail: { state: "available", reason: null },
        semantic_search: { state: "unavailable", reason: "embedding_unavailable" },
        hybrid_search: { state: "degraded", reason: "embedding_unavailable" },
        text_import: { state: "unavailable", reason: "embedding_unavailable" },
        ocr_import: { state: "unavailable", reason: "embedding_unavailable" },
        index_publication: { state: "unavailable", reason: "embedding_unavailable" },
      },
    } as StatusBody
    const lifecycle = { ...running, last_exit: "heartbeat_timeout" as const }
    const markup = renderToStaticMarkup(createElement(CapabilityMatrix, { lifecycle, status, runtimeView: "trusted" }))
    expect(markup).toContain("语义运行时 · unavailable · 完整性无效")
    expect(markup).toContain("关键词检索 · available")
    expect(markup).toContain("混合检索 · degraded · 语义运行时不可用")
    expect(markup).toContain("上次退出 · 心跳超时")
    expect(markup).toContain("状态原因 · 控制面就绪")
  })

  it("shows a bounded blocked reason without a stale service snapshot", () => {
    const lifecycle = { ...running, state: "blocked" as const, transition_reason: "runtime_integrity" as const }
    const markup = renderToStaticMarkup(createElement(CapabilityMatrix, { lifecycle, status: null, runtimeView: "service_unknown" }))
    expect(markup).toContain("进程 blocked")
    expect(markup).toContain("状态原因 · 运行时完整性失败")
    expect(markup).toContain("旧健康快照不会继续授权任何操作")
  })
})
