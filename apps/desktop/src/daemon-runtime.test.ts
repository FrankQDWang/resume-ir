import { describe, expect, it } from "vitest"

import {
  daemonRetryControl,
  deletionReceiptUncertainPresentation,
  hasActiveManagedRootScan,
  managedRootsReadFailureAfterRefresh,
  rootsAfterDeletionAccepted,
  sourcePanelBanner,
  startSerialManagedRootsPolling,
} from "./daemon-runtime"
import type { SourceRoot } from "./daemon"
import {
  captureLifecycleReadability,
  initialLifecycleReadability,
  invalidateLifecycleReadability,
  lifecycleReadabilityIsCurrent,
  observeReadableLifecycle,
  type DaemonLifecycleSnapshot,
} from "./runtime-state"

function lifecycle(
  state: "circuit_open" | "blocked",
  retryAfterMs: number | null,
): DaemonLifecycleSnapshot {
  return {
    schema_version: "resume-ir.desktop-daemon-lifecycle.v2",
    state,
    transition_reason: state === "circuit_open" ? "restart_budget_exhausted" : "runtime_integrity",
    generation: 3,
    automatic_restart_attempt: state === "circuit_open" ? 5 : 0,
    automatic_restart_limit: 5,
    retry_after_ms: retryAfterMs,
    heartbeat_failures: 0,
    last_exit: state === "circuit_open" ? "child_exited" : null,
  }
}

describe("daemon retry control", () => {
  it("keeps circuit-open retry disabled until the supervisor reaches zero", () => {
    expect(daemonRetryControl(lifecycle("circuit_open", 1_001))).toEqual({
      disabled: true,
      label: "2 秒后可重试",
    })
    expect(daemonRetryControl(lifecycle("circuit_open", 0))).toEqual({
      disabled: false,
      label: "重新检测并启动",
    })
  })

  it("bounds the visible countdown and leaves blocked recovery enabled", () => {
    expect(daemonRetryControl(lifecycle("circuit_open", Number.MAX_SAFE_INTEGER))).toEqual({
      disabled: true,
      label: "300 秒后可重试",
    })
    expect(daemonRetryControl(lifecycle("blocked", null))).toEqual({
      disabled: false,
      label: "重新检测并启动",
    })
  })
})

describe("managed source-root read recovery", () => {
  it("clears ordinary and overload read failures after a valid refresh", () => {
    expect(managedRootsReadFailureAfterRefresh("error")).toBe("error")
    expect(managedRootsReadFailureAfterRefresh("success")).toBeNull()
    expect(managedRootsReadFailureAfterRefresh("overload")).toBe("overload")
    expect(managedRootsReadFailureAfterRefresh("success")).toBeNull()
  })

  it("keeps source-list failures separate from other import operation messages", () => {
    expect(sourcePanelBanner("selected", "监控已恢复", "error")).toEqual({
      state: "error",
      message: "无法读取本地授权目录记录",
    })
    expect(sourcePanelBanner("selected", "监控已恢复", "overload")).toEqual({
      state: "overload",
      message: "授权目录读取入口繁忙，请稍后重试",
    })
    expect(sourcePanelBanner("error", "daemon 未接受目录监控操作，可重试读取状态", null)).toEqual({
      state: "error",
      message: "daemon 未接受目录监控操作，可重试读取状态",
    })
  })

  it("projects a 202 receipt as deleting without changing an unrelated root", () => {
    const root = (id: string, watcher_state: SourceRoot["watcher_state"]): SourceRoot => ({
      root_id: `root-${id.repeat(32)}`,
      display_label: `Synthetic ${id.toUpperCase()}`,
      state: "active",
      watcher_state,
      current_counts: { discovered: 0, searchable: 0, non_resume: 0, needs_review: 0, ocr: 0, failed: 0 },
      last_scan: null,
    })
    const roots = [root("a", "active"), root("b", "paused")]

    expect(rootsAfterDeletionAccepted(roots, roots[0].root_id)).toEqual([
      { ...roots[0], state: "deleting" },
      roots[1],
    ])
  })

  it("uses an authoritative-refresh state when the delete receipt is uncertain", () => {
    expect(deletionReceiptUncertainPresentation()).toEqual({
      state: "submitting",
      message: "目录删除接收状态未确认，正在重新读取授权目录",
    })
  })
})

describe("active managed-root refresh", () => {
  const activeRoot = (phase: NonNullable<SourceRoot["last_scan"]>["phase"]): SourceRoot => ({
    root_id: "root-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    display_label: "Synthetic A",
    state: "active",
    watcher_state: "active",
    current_counts: { discovered: 10, searchable: 4, non_resume: 0, needs_review: 0, ocr: 0, failed: 0 },
    last_scan: {
      scan_id: "scan-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      trigger: "manual",
      phase,
      completeness: phase === "complete" ? "complete" : "unknown",
      counts: { discovered: 10, searchable: 4, non_resume: 0, needs_review: 0, ocr: 0, failed: 0, ignored: 0, processed: 4, total: 10, errors: 0 },
      rate_per_second: 2,
      eta_seconds: 3,
      started_at_seconds: 1,
      updated_at_seconds: 2,
      completed_at_seconds: phase === "complete" ? 2 : null,
    },
  })

  it("requests fast refresh only for an active scan", () => {
    expect(hasActiveManagedRootScan([activeRoot("parsing")])).toBe(true)
    expect(hasActiveManagedRootScan([activeRoot("complete")])).toBe(false)
    expect(hasActiveManagedRootScan([])).toBe(false)
  })

  it("serializes one-second refreshes without overlap", async () => {
    const timers = new Map<number, { callback: () => void; delayMs: number }>()
    let nextTimer = 1
    let refreshes = 0
    let release!: () => void
    const firstRefresh = new Promise<void>((resolve) => { release = resolve })
    const stop = startSerialManagedRootsPolling({
      refresh: async () => {
        refreshes += 1
        if (refreshes === 1) await firstRefresh
      },
      clock: {
        setTimeout: (callback, delayMs) => {
          const timer = nextTimer++
          timers.set(timer, { callback, delayMs })
          return timer
        },
        clearTimeout: (timer) => { timers.delete(timer) },
      },
    })

    expect([...timers.values()].map(({ delayMs }) => delayMs)).toEqual([1000])
    const firstTimer = [...timers.entries()][0]
    timers.delete(firstTimer[0])
    firstTimer[1].callback()
    expect(refreshes).toBe(1)
    expect(timers.size).toBe(0)

    release()
    await Promise.resolve()
    await Promise.resolve()
    expect([...timers.values()].map(({ delayMs }) => delayMs)).toEqual([1000])
    stop()
    expect(timers.size).toBe(0)
  })
})

describe("lifecycle readability authority", () => {
  it("rejects a deferred status response after a lifecycle bridge error", async () => {
    let readability = observeReadableLifecycle(initialLifecycleReadability())
    const requested = captureLifecycleReadability(readability)
    expect(requested).not.toBeNull()

    let releaseStatus!: () => void
    const deferredStatus = new Promise<void>((resolve) => { releaseStatus = resolve })
    const mayCommit = deferredStatus.then(() =>
      lifecycleReadabilityIsCurrent(readability, requested!),
    )

    readability = invalidateLifecycleReadability(readability)
    releaseStatus()

    expect(await mayCommit).toBe(false)
    expect(captureLifecycleReadability(readability)).toBeNull()
  })

  it("accepts status only within one unchanged readable observation", () => {
    const readability = observeReadableLifecycle(initialLifecycleReadability())
    const requested = captureLifecycleReadability(readability)
    expect(requested).not.toBeNull()
    expect(lifecycleReadabilityIsCurrent(readability, requested!)).toBe(true)
  })
})
