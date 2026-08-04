import { describe, expect, it } from "vitest"

import type { SourceRoot } from "./daemon"
import { importProgressPresentation } from "./import-progress"

function root(overrides: Partial<SourceRoot> = {}): SourceRoot {
  return {
    root_id: "root-00000000000000000000000000000000",
    display_label: "合成简历",
    state: "active",
    watcher_state: "active",
    current_counts: {
      discovered: 10,
      searchable: 8,
      non_resume: 1,
      needs_review: 0,
      ocr: 1,
      failed: 0,
    },
    last_scan: {
      scan_id: "scan-00000000000000000000000000000000",
      trigger: "manual",
      phase: "parsing",
      completeness: "unknown",
      counts: {
        discovered: 10,
        searchable: 8,
        non_resume: 1,
        needs_review: 0,
        ocr: 1,
        failed: 0,
        ignored: 0,
        processed: 8,
        total: 10,
        errors: 0,
      },
      rate_per_second: 2,
      eta_seconds: 1,
      started_at_seconds: 1,
      updated_at_seconds: 2,
      completed_at_seconds: null,
    },
    ...overrides,
  }
}

describe("truthful import progress presentation", () => {
  it("treats a nonzero searchable count during import as partial only", () => {
    const presentation = importProgressPresentation(root(), "available")
    expect(presentation.observedPercent).toBe(80)
    expect(presentation.stageMessage).toBe("正在解析和处理")
    expect(presentation.keywordMessage).toBe("关键词检索已部分可用")
  })

  it("reports only the exact phase supplied by the backend", () => {
    const expected = {
      queued: "等待扫描",
      discovering: "正在发现文件",
      fingerprinting: "正在核对文件变更",
      classifying: "正在识别简历与其他文件",
      parsing: "正在解析和处理",
      ocr: "正在处理 OCR 文件",
      publishing: "正在发布关键词和语义索引",
    } as const
    for (const [phase, message] of Object.entries(expected)) {
      const candidate = root({ last_scan: { ...root().last_scan!, phase: phase as keyof typeof expected } })
      expect(importProgressPresentation(candidate, "available").stageMessage).toBe(message)
    }
  })

  it("does not claim complete readiness while OCR continues after a complete scan", () => {
    const candidate = root({
      last_scan: {
        ...root().last_scan!,
        phase: "complete",
        completeness: "complete",
        counts: { ...root().last_scan!.counts, processed: 10 },
        completed_at_seconds: 2,
      },
    })
    const presentation = importProgressPresentation(candidate, "available")
    expect(presentation.stageMessage).toContain("OCR 正在后台继续")
    expect(presentation.keywordState).toBe("partial")
  })

  it("requires every strict gate before claiming all keyword search is available", () => {
    const complete = root({
      current_counts: { ...root().current_counts, ocr: 0 },
      last_scan: {
        ...root().last_scan!,
        phase: "complete",
        completeness: "complete",
        counts: { ...root().last_scan!.counts, ocr: 0, processed: 10 },
        completed_at_seconds: 2,
      },
    })
    expect(importProgressPresentation(complete, "available").keywordMessage)
      .toBe("关键词检索全部可用")
    expect(importProgressPresentation(complete, "degraded").keywordState).toBe("partial")
    expect(importProgressPresentation({ ...complete, last_scan: { ...complete.last_scan!, completeness: "partial" } }, "available").keywordState).toBe("partial")
    expect(importProgressPresentation({ ...complete, last_scan: { ...complete.last_scan!, counts: { ...complete.last_scan!.counts, processed: 9 } } }, "available").keywordState).toBe("partial")
  })

  it("never turns partial or failed terminal scans into complete readiness", () => {
    for (const phase of ["partial", "failed"] as const) {
      const candidate = root({
        current_counts: { ...root().current_counts, ocr: 0 },
        last_scan: {
          ...root().last_scan!,
          phase,
          completeness: "partial",
          counts: { ...root().last_scan!.counts, ocr: 0, processed: 10 },
          completed_at_seconds: 2,
        },
      })
      expect(importProgressPresentation(candidate, "available").keywordState).toBe("partial")
    }
  })

  it("does not call an empty completed directory keyword-ready", () => {
    const empty = root({
      current_counts: { discovered: 0, searchable: 0, non_resume: 0, needs_review: 0, ocr: 0, failed: 0 },
      last_scan: {
        ...root().last_scan!,
        phase: "complete",
        completeness: "complete",
        counts: { ...root().last_scan!.counts, discovered: 0, searchable: 0, processed: 0, total: 0 },
        completed_at_seconds: 2,
      },
    })
    expect(importProgressPresentation(empty, "available").keywordState).toBe("waiting")
  })
})
