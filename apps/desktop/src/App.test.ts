import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it } from "vitest"

import { IndexServiceSummary, indexServicePresentation } from "./App"

describe("visible index service state", () => {
  it("shows a runtime invariant as blocked repair with a diagnostic action", () => {
    const expected = {
      title: "索引修复已阻塞",
      message: "daemon 已连接，索引修复已阻塞，请导出诊断",
    }
    expect(indexServicePresentation("degraded", "runtime_invariant")).toEqual(expected)

    const markup = renderToStaticMarkup(createElement(IndexServiceSummary, {
      lifecycle: {
        schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
        state: "ready",
        generation: 3,
        restart_attempt: 0,
        restart_budget: 5,
        retry_delay_ms: null,
        consecutive_heartbeat_failures: 0,
        blocked_reason: null,
        last_exit: null,
        restart_ledger_reason: null,
      },
      service: "degraded",
      status: {
        repair_reason: "runtime_invariant",
        repair_progress: { phase: "blocked", attempt: 5, max_attempts: 5, retry_after_ms: null, last_error_kind: "fulltext_failure" },
        searchable_documents: 0,
        ocr_queue_depth: 0,
      },
      searchablePercent: 0,
      connectionMessage: "stale generic message",
    }))
    expect(markup).toContain(expected.title)
    expect(markup).toContain("请导出诊断")
    expect(markup).not.toContain("stale generic message")
  })

  it("keeps active repair, blocked source recovery, and generic degradation distinct", () => {
    expect(indexServicePresentation("repairing", "migration_rebuild")).toEqual({
      title: "索引修复中",
      message: "daemon 已连接，索引正在修复",
    })
    expect(indexServicePresentation("degraded", "source_unavailable")).toEqual({
      title: "索引修复已阻塞",
      message: "daemon 已连接，来源不可用，请恢复来源磁盘连接或文件权限",
    })
    expect(indexServicePresentation("degraded", null)).toEqual({
      title: "索引能力降级",
      message: "daemon 已连接，索引能力降级",
    })
    expect(indexServicePresentation("repairing", "artifact_unavailable", {
      phase: "retry_wait",
      attempt: 2,
      max_attempts: 5,
      retry_after_ms: 3_100,
      last_error_kind: "fulltext_publication_busy",
    })).toEqual({
      title: "索引修复等待重试",
      message: "第 2/5 次修复未完成，4 秒后继续",
    })

    const blockedSourceMarkup = renderToStaticMarkup(createElement(IndexServiceSummary, {
      lifecycle: {
        schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
        state: "ready",
        generation: 4,
        restart_attempt: 0,
        restart_budget: 5,
        retry_delay_ms: null,
        consecutive_heartbeat_failures: 0,
        blocked_reason: null,
        last_exit: null,
        restart_ledger_reason: null,
      },
      service: "degraded",
      status: {
        repair_reason: "source_unavailable",
        repair_progress: { phase: "source_unavailable", attempt: null, max_attempts: null, retry_after_ms: null, last_error_kind: null },
        searchable_documents: 0,
        ocr_queue_depth: 0,
      },
      searchablePercent: 0,
      connectionMessage: "stale generic message",
    }))
    expect(blockedSourceMarkup).toContain("索引修复已阻塞")
    expect(blockedSourceMarkup).toContain("请恢复来源磁盘连接或文件权限")
    expect(blockedSourceMarkup).not.toContain("stale generic message")
  })
})
