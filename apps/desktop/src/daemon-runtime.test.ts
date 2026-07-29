import { describe, expect, it } from "vitest"

import {
  daemonRetryControl,
  managedRootsReadFailureAfterRefresh,
  sourcePanelBanner,
} from "./daemon-runtime"
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
