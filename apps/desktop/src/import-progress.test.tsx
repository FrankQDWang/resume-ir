import { describe, expect, it } from "vitest"

import type { SourceRoot } from "./daemon"
import {
  hasActiveManagedRootScan,
  importProgressPresentation,
  rootCardHeadingStatus,
  type ImportProgressSignals,
} from "./import-progress"

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

function signals(overrides: Partial<ImportProgressSignals> = {}): ImportProgressSignals {
  return {
    keywordCapabilityState: "available",
    semanticCapabilityState: "unavailable",
    embeddingQueueDepth: 0,
    ...overrides,
  }
}

describe("truthful import progress presentation", () => {
  it("shows percent and searchable count while keyword is only partially ready", () => {
    const presentation = importProgressPresentation(root(), signals())
    expect(presentation.observedPercent).toBe(80)
    expect(presentation.stageMessage).toBe("解析中 80% · 关键词检索已部分可用（已可搜 8）")
    expect(presentation.stageMessage).not.toContain("全部可用")
    expect(presentation.scanActionLabel).toBe("扫描中")
    expect(presentation.progressValueLabel).toBe("80% · 约 1 秒")
  })

  it("keeps the active stage with percent when keyword capability is degraded", () => {
    const presentation = importProgressPresentation(root(), signals({ keywordCapabilityState: "degraded" }))
    expect(presentation.stageMessage).toBe("解析中 80%")
    expect(presentation.stageMessage).not.toContain("关键词检索")
  })

  it("keeps the active stage with percent before any searchable documents exist", () => {
    const waiting = root({
      current_counts: { ...root().current_counts, searchable: 0 },
      last_scan: {
        ...root().last_scan!,
        counts: { ...root().last_scan!.counts, searchable: 0 },
      },
    })
    expect(importProgressPresentation(waiting, signals()).stageMessage).toBe("解析中 80%")
  })

  it("uses a frontend publishing hint when counters stall during parsing", () => {
    const presentation = importProgressPresentation(root(), signals({ indexPublishingHint: true }))
    expect(presentation.stageMessage).toBe("索引发布中 80% · 关键词检索已部分可用（已可搜 8）")
  })

  it("appends coarse embedding progress during an active scan", () => {
    const presentation = importProgressPresentation(root(), signals({ embeddingQueueDepth: 3 }))
    expect(presentation.stageMessage).toBe("解析中 80% · 关键词检索已部分可用（已可搜 8） · 语义索引生成中")
  })

  it("reports backend phases with observed percent while active", () => {
    const expected = {
      queued: "等待扫描 80%",
      discovering: "发现文件 80%",
      fingerprinting: "核对变更 80%",
      classifying: "分类中 80%",
      parsing: "解析中 80%",
      ocr: "OCR 处理中 80%",
      publishing: "发布索引 80%",
    } as const
    for (const [phase, message] of Object.entries(expected)) {
      const candidate = root({
        current_counts: { ...root().current_counts, searchable: 0 },
        last_scan: {
          ...root().last_scan!,
          phase: phase as keyof typeof expected,
          counts: { ...root().last_scan!.counts, searchable: 0 },
        },
      })
      const presentation = importProgressPresentation(candidate, signals())
      expect(presentation.stageMessage).toBe(message)
    }
  })

  it("appends partial keyword readiness to every observed active phase", () => {
    for (const phase of ["queued", "discovering", "parsing", "publishing"] as const) {
      const candidate = root({ last_scan: { ...root().last_scan!, phase } })
      const presentation = importProgressPresentation(candidate, signals())
      expect(presentation.stageMessage).toContain("关键词检索已部分可用（已可搜 8）")
      expect(presentation.stageMessage).not.toContain("全部可用")
    }
  })

  it("owns active-phase classification for every managed-root consumer", () => {
    expect(hasActiveManagedRootScan([root()])).toBe(true)
    expect(hasActiveManagedRootScan([root({
      last_scan: { ...root().last_scan!, phase: "complete" },
    })])).toBe(false)
    expect(hasActiveManagedRootScan([])).toBe(false)
  })

  it("keeps the card heading on short lifecycle labels instead of progress sentences", () => {
    expect(rootCardHeadingStatus(root())).toBe("解析中")
    expect(rootCardHeadingStatus(root({
      last_scan: {
        ...root().last_scan!,
        phase: "complete",
        completeness: "complete",
        counts: { ...root().last_scan!.counts, processed: 10 },
        completed_at_seconds: 2,
      },
    }))).toBe("持续监控中")
    expect(rootCardHeadingStatus(root({ state: "deleting" }))).toBe("正在删除本地数据")
    expect(rootCardHeadingStatus(root({ watcher_state: "paused" }))).toBe("监控已暂停")
  })

  it("claims keyword-complete after scan complete even while OCR continues", () => {
    const candidate = root({
      last_scan: {
        ...root().last_scan!,
        phase: "complete",
        completeness: "complete",
        counts: { ...root().last_scan!.counts, processed: 10 },
        completed_at_seconds: 2,
      },
    })
    const presentation = importProgressPresentation(candidate, signals({
      semanticCapabilityState: "available",
    }))
    expect(presentation.stageMessage).toBe("关键词检索全部可用 · OCR 后台继续 · 语义检索可用")
  })

  it("uses the main stage sentence only after every keyword-ready gate passes", () => {
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
    expect(importProgressPresentation(complete, signals({ semanticCapabilityState: "available" })).stageMessage)
      .toBe("关键词检索全部可用 · 语义检索可用")
    expect(importProgressPresentation(complete, signals({
      keywordCapabilityState: "degraded",
      semanticCapabilityState: "available",
    })).stageMessage).toBe("本轮扫描完成")
    expect(importProgressPresentation({ ...complete, last_scan: { ...complete.last_scan!, completeness: "partial" } }, signals()).stageMessage).toBe("本轮扫描完成")
    expect(importProgressPresentation({ ...complete, last_scan: { ...complete.last_scan!, counts: { ...complete.last_scan!.counts, processed: 9 } } }, signals()).stageMessage).toBe("本轮扫描完成")
  })

  it("prefers coarse embedding progress over semantic-available after keyword-complete", () => {
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
    expect(importProgressPresentation(complete, signals({
      semanticCapabilityState: "available",
      embeddingQueueDepth: 2,
    })).stageMessage).toBe("关键词检索全部可用 · 语义索引生成中")
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
      expect(importProgressPresentation(candidate, signals()).stageMessage)
        .toBe(phase === "partial" ? "本轮扫描不完整" : "本轮扫描失败")
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
    expect(importProgressPresentation(empty, signals()).stageMessage).toBe("本轮扫描完成")
  })
})
