import { describe, expect, it, vi } from "vitest"

import {
  blockedReasonMessage,
  detailReadMayContinue,
  isDaemonLifecycleSnapshot,
  lifecyclePollDelayMs,
  reconcileResultFreshness,
  serviceStateFromStatus,
  startSerialLifecyclePolling,
  type DaemonLifecycleSnapshot,
} from "./runtime-state"

const lifecycle = (
  state: DaemonLifecycleSnapshot["state"],
  generation = 3,
): DaemonLifecycleSnapshot => ({
  schema_version: "resume-ir.desktop-daemon-lifecycle.v1",
  state,
  generation,
  restart_attempt: 0,
  restart_budget: 5,
  retry_delay_ms: null,
  consecutive_heartbeat_failures: 0,
  blocked_reason: null,
  last_exit: null,
})

describe("desktop runtime axes", () => {
  it("polls ready slowly and every non-ready lifecycle state quickly", () => {
    expect(lifecyclePollDelayMs("ready")).toBe(5000)
    for (const state of ["starting", "recovering", "circuit_open", "blocked"] as const) {
      expect(lifecyclePollDelayMs(state)).toBe(1000)
    }
  })

  it("keeps lifecycle separate from daemon service health", () => {
    expect(serviceStateFromStatus({ httpStatus: 200, status: "ready" })).toBe("ready")
    expect(serviceStateFromStatus({ httpStatus: 200, status: "repairing" })).toBe("repairing")
    expect(serviceStateFromStatus({ httpStatus: 503, status: "unavailable" })).toBe("degraded")
  })

  it("explains each fail-closed blocked reason without collapsing them", () => {
    const messages = [
      blockedReasonMessage("configuration_invalid"),
      blockedReasonMessage("runtime_integrity"),
      blockedReasonMessage("protocol_mismatch"),
      blockedReasonMessage("ownership_conflict"),
      blockedReasonMessage("supervisor_unavailable"),
    ]
    expect(new Set(messages).size).toBe(messages.length)
    expect(messages.join(" ")).not.toContain("undefined")
  })

  it("marks results interrupted across recovery and stale only for a later visible epoch", () => {
    expect(reconcileResultFreshness({
      current: "current",
      hasResults: true,
      resultGeneration: 3,
      resultVisibleEpoch: 7,
      lifecycle: lifecycle("recovering"),
      serviceVisibleEpoch: 7,
    })).toBe("interrupted")
    expect(reconcileResultFreshness({
      current: "current",
      hasResults: true,
      resultGeneration: 3,
      resultVisibleEpoch: 7,
      lifecycle: lifecycle("ready"),
      serviceVisibleEpoch: 8,
    })).toBe("stale")
    expect(reconcileResultFreshness({
      current: "stale",
      hasResults: true,
      resultGeneration: 3,
      resultVisibleEpoch: 7,
      lifecycle: lifecycle("ready"),
      serviceVisibleEpoch: 7,
    })).toBe("stale")
  })

  it("never continues a detail page read after lifecycle or generation changes", () => {
    expect(detailReadMayContinue(lifecycle("ready"), 3)).toBe(true)
    expect(detailReadMayContinue(lifecycle("recovering"), 3)).toBe(false)
    expect(detailReadMayContinue(lifecycle("ready", 4), 3)).toBe(false)
  })

  it("rejects unknown lifecycle enums and out-of-budget counters", () => {
    expect(isDaemonLifecycleSnapshot(lifecycle("ready"))).toBe(true)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("recovering"), last_exit: "control_plane_failure" })).toBe(true)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("ready"), blocked_reason: "private-path" })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("recovering"), last_exit: "raw-stderr" })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("blocked"), blocked_reason: null })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("ready"), restart_attempt: 6 })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("recovering"), retry_delay_ms: 300_001 })).toBe(false)
  })

  it("serializes focus refreshes and schedules from the completed lifecycle state", async () => {
    const pending: Array<(snapshot: DaemonLifecycleSnapshot) => void> = []
    const scheduled: Array<{ callback: () => void; delayMs: number }> = []
    const focus: { listener: (() => void) | null } = { listener: null }
    const readSnapshot = vi.fn(() => new Promise<DaemonLifecycleSnapshot>((resolve) => pending.push(resolve)))
    const stop = startSerialLifecyclePolling({
      readSnapshot,
      onSnapshot: vi.fn(),
      onError: vi.fn(),
      clock: {
        setTimeout: (callback, delayMs) => { scheduled.push({ callback, delayMs }); return scheduled.length },
        clearTimeout: vi.fn(),
      },
      focusEvents: {
        addFocusListener: (listener) => { focus.listener = listener },
        removeFocusListener: vi.fn(),
      },
    })

    expect(readSnapshot).toHaveBeenCalledTimes(1)
    focus.listener?.()
    expect(readSnapshot).toHaveBeenCalledTimes(1)

    pending.shift()?.(lifecycle("recovering"))
    await vi.waitFor(() => expect(scheduled.at(-1)?.delayMs).toBe(0))
    scheduled.at(-1)?.callback()
    expect(readSnapshot).toHaveBeenCalledTimes(2)

    pending.shift()?.(lifecycle("ready"))
    await vi.waitFor(() => expect(scheduled.at(-1)?.delayMs).toBe(5000))
    stop()
  })
})
