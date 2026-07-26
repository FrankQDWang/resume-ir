import { describe, expect, it, vi } from "vitest"

import {
  blockedReasonMessage,
  captureDaemonActionAuthority,
  daemonActionAuthorityIsCurrent,
  detailReadMayContinue,
  initialDaemonActionAuthority,
  isDaemonLifecycleSnapshot,
  lifecycleInvalidatesStatusAuthority,
  lifecyclePollDelayMs,
  reconcileResultFreshness,
  revokeDaemonActionAuthority,
  serviceStateFromStatus,
  startSerialLifecyclePolling,
  statusAuthorityIsCurrent,
  trustDaemonActionAuthority,
  type DaemonLifecycleSnapshot,
} from "./runtime-state"

const lifecycle = (
  state: DaemonLifecycleSnapshot["state"],
  generation = 3,
): DaemonLifecycleSnapshot => ({
  schema_version: "resume-ir.desktop-daemon-lifecycle.v2",
  state,
  transition_reason: state === "running" ? "control_plane_ready" : state === "retry_wait" ? "child_exited" : state === "circuit_open" ? "restart_budget_exhausted" : state === "blocked" ? "runtime_integrity" : generation === 0 ? "initial_start" : "automatic_retry",
  generation,
  automatic_restart_attempt: state === "circuit_open" ? 5 : 0,
  automatic_restart_limit: 5,
  retry_after_ms: state === "retry_wait" || state === "circuit_open" ? 250 : null,
  heartbeat_failures: 0,
  last_exit: state === "retry_wait" || state === "circuit_open" ? "child_exited" : null,
})

describe("desktop runtime axes", () => {
  it("polls running slowly and every non-running lifecycle state quickly", () => {
    expect(lifecyclePollDelayMs("running")).toBe(5000)
    for (const state of ["starting", "retry_wait", "circuit_open", "blocked"] as const) {
      expect(lifecyclePollDelayMs(state)).toBe(1000)
    }
  })

  it("keeps lifecycle separate from core service health", () => {
    expect(serviceStateFromStatus({ httpStatus: 200, status: "ready" })).toBe("ready")
    expect(serviceStateFromStatus({ httpStatus: 200, status: "initializing" })).toBe("initializing")
    expect(serviceStateFromStatus({ httpStatus: 200, status: "repairing" })).toBe("repairing")
    expect(serviceStateFromStatus({ httpStatus: 503, status: "blocked" })).toBe("blocked")
    expect(serviceStateFromStatus({ httpStatus: 503, status: "unknown" })).toBe("unknown")
  })

  it("explains each fail-closed blocked reason", () => {
    const messages = ["configuration_invalid", "runtime_integrity", "protocol_mismatch", "ownership_conflict", "supervisor_unavailable"].map((reason) => blockedReasonMessage(reason as DaemonLifecycleSnapshot["transition_reason"]))
    expect(new Set(messages).size).toBe(messages.length)
  })

  it("invalidates results and detail across a supervisor generation", () => {
    expect(reconcileResultFreshness({ current: "current", hasResults: true, resultGeneration: 3, resultVisibleEpoch: 7, lifecycle: lifecycle("retry_wait"), serviceVisibleEpoch: 7 })).toBe("interrupted")
    expect(reconcileResultFreshness({ current: "current", hasResults: true, resultGeneration: 3, resultVisibleEpoch: 7, lifecycle: lifecycle("running"), serviceVisibleEpoch: 8 })).toBe("stale")
    expect(detailReadMayContinue(lifecycle("running"), 3)).toBe(true)
    expect(detailReadMayContinue(lifecycle("retry_wait"), 3)).toBe(false)
    expect(detailReadMayContinue(lifecycle("running", 4), 3)).toBe(false)
  })

  it("revokes status authority when polling skips directly to a new running generation", () => {
    const oldRunning = lifecycle("running", 3)
    const newRunning = lifecycle("running", 4)
    expect(statusAuthorityIsCurrent(oldRunning, 3)).toBe(true)
    expect(statusAuthorityIsCurrent(newRunning, 3)).toBe(false)
    expect(lifecycleInvalidatesStatusAuthority(oldRunning, newRunning, 3)).toBe(true)
    expect(lifecycleInvalidatesStatusAuthority(oldRunning, oldRunning, 3)).toBe(false)
    expect(lifecycleInvalidatesStatusAuthority(oldRunning, lifecycle("retry_wait", 3), 3)).toBe(true)
  })

  it("revokes synchronous daemon action tokens until a fresh status grants authority", () => {
    const running = lifecycle("running", 3)
    let authority = initialDaemonActionAuthority()
    expect(captureDaemonActionAuthority(authority, running)).toBeNull()

    authority = trustDaemonActionAuthority(authority, 3)
    const token = captureDaemonActionAuthority(authority, running)
    expect(token).toEqual({ epoch: 1, generation: 3 })
    expect(daemonActionAuthorityIsCurrent(authority, running, token!)).toBe(true)

    authority = revokeDaemonActionAuthority(authority)
    expect(captureDaemonActionAuthority(authority, running)).toBeNull()
    expect(daemonActionAuthorityIsCurrent(authority, running, token!)).toBe(false)

    authority = trustDaemonActionAuthority(authority, 3)
    const restored = captureDaemonActionAuthority(authority, running)
    expect(restored).toEqual({ epoch: 3, generation: 3 })
    expect(daemonActionAuthorityIsCurrent(authority, lifecycle("running", 4), restored!)).toBe(false)
  })

  it("accepts only exact lifecycle v2 state-reason-field combinations", () => {
    expect(isDaemonLifecycleSnapshot(lifecycle("running"))).toBe(true)
    expect(isDaemonLifecycleSnapshot(lifecycle("retry_wait"))).toBe(true)
    expect(isDaemonLifecycleSnapshot(lifecycle("circuit_open"))).toBe(true)
    expect(isDaemonLifecycleSnapshot(lifecycle("blocked"))).toBe(true)
    expect(isDaemonLifecycleSnapshot(lifecycle("starting", 0))).toBe(true)

    expect(isDaemonLifecycleSnapshot({ ...lifecycle("running"), schema_version: "resume-ir.desktop-daemon-lifecycle.v1" })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("running"), private_debug: true })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("running"), retry_after_ms: 1 })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("running"), generation: 0 })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("retry_wait"), retry_after_ms: null })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("retry_wait"), transition_reason: "heartbeat_timeout", last_exit: "child_exited" })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("circuit_open"), automatic_restart_attempt: 4 })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("blocked"), transition_reason: "child_exited" })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("starting", 0), transition_reason: "initial_start", generation: 1 })).toBe(false)
    expect(isDaemonLifecycleSnapshot({ ...lifecycle("running"), heartbeat_failures: 4 })).toBe(false)
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
    focus.listener?.()
    expect(readSnapshot).toHaveBeenCalledTimes(1)
    pending.shift()?.(lifecycle("retry_wait"))
    await vi.waitFor(() => expect(scheduled.at(-1)?.delayMs).toBe(0))
    scheduled.at(-1)?.callback()
    pending.shift()?.(lifecycle("running"))
    await vi.waitFor(() => expect(scheduled.at(-1)?.delayMs).toBe(5000))
    stop()
  })
})
