export type DaemonLifecycle = "starting" | "ready" | "recovering" | "circuit_open" | "blocked"
export type DaemonService = "ready" | "degraded" | "repairing"
export type ResultFreshness = "current" | "stale" | "interrupted"

export type DaemonBlockedReason =
  | "configuration_invalid"
  | "runtime_integrity"
  | "protocol_mismatch"
  | "ownership_conflict"
  | "supervisor_unavailable"

export type DaemonExitClass =
  | "child_exited"
  | "startup_timeout"
  | "heartbeat_timeout"
  | "start_failed"
  | "control_plane_failure"

export interface DaemonLifecycleSnapshot {
  schema_version: "resume-ir.desktop-daemon-lifecycle.v1"
  state: DaemonLifecycle
  generation: number
  restart_attempt: number
  restart_budget: number
  retry_delay_ms: number | null
  consecutive_heartbeat_failures: number
  blocked_reason: DaemonBlockedReason | null
  last_exit: DaemonExitClass | null
}

export function blockedReasonMessage(reason: DaemonBlockedReason | null): string {
  switch (reason) {
    case "configuration_invalid": return "daemon 配置无效，需修正本地配置后重试"
    case "runtime_integrity": return "daemon 运行时完整性校验失败，已停止自动重启"
    case "protocol_mismatch": return "桌面端与 daemon 协议不匹配，需安装同一版本"
    case "ownership_conflict": return "已有其他进程持有 daemon 数据目录，已拒绝抢占"
    case "supervisor_unavailable": return "桌面原生监督器不可用，需重新启动应用"
    case null: return "daemon 启动已阻止"
  }
}

export function lifecyclePollDelayMs(state: DaemonLifecycle): 1000 | 5000 {
  return state === "ready" ? 5000 : 1000
}

export interface LifecyclePollClock {
  setTimeout(callback: () => void, delayMs: number): number
  clearTimeout(timer: number): void
}

export interface LifecycleFocusEvents {
  addFocusListener(listener: () => void): void
  removeFocusListener(listener: () => void): void
}

export function startSerialLifecyclePolling(input: {
  readSnapshot: () => Promise<DaemonLifecycleSnapshot>
  onSnapshot: (snapshot: DaemonLifecycleSnapshot) => void | Promise<void>
  onError: (error: unknown) => void
  clock: LifecyclePollClock
  focusEvents: LifecycleFocusEvents
}): () => void {
  let stopped = false
  let inFlight = false
  let immediateQueued = false
  let timer: number | null = null

  const schedule = (delayMs: number) => {
    if (stopped) return
    if (timer !== null) input.clock.clearTimeout(timer)
    timer = input.clock.setTimeout(() => { void poll() }, delayMs)
  }
  const poll = async () => {
    if (stopped) return
    if (inFlight) {
      immediateQueued = true
      return
    }
    if (timer !== null) {
      input.clock.clearTimeout(timer)
      timer = null
    }
    inFlight = true
    let delayMs = 1000
    try {
      const snapshot = await input.readSnapshot()
      if (stopped) return
      delayMs = lifecyclePollDelayMs(snapshot.state)
      await input.onSnapshot(snapshot)
    } catch (error) {
      if (!stopped) input.onError(error)
    } finally {
      inFlight = false
      if (!stopped) {
        if (immediateQueued) {
          immediateQueued = false
          schedule(0)
        } else {
          schedule(delayMs)
        }
      }
    }
  }
  const refreshOnFocus = () => {
    if (timer !== null) input.clock.clearTimeout(timer)
    timer = null
    void poll()
  }

  input.focusEvents.addFocusListener(refreshOnFocus)
  void poll()
  return () => {
    stopped = true
    if (timer !== null) input.clock.clearTimeout(timer)
    input.focusEvents.removeFocusListener(refreshOnFocus)
  }
}

export function serviceStateFromStatus(input: {
  httpStatus: number
  status: "ready" | "degraded" | "repairing" | "unavailable"
}): DaemonService {
  if (input.httpStatus >= 200 && input.httpStatus < 300 && input.status === "ready") return "ready"
  if (input.status === "repairing") return "repairing"
  return "degraded"
}

export function reconcileResultFreshness(input: {
  current: ResultFreshness
  hasResults: boolean
  resultGeneration: number | null
  resultVisibleEpoch: number | null
  lifecycle: DaemonLifecycleSnapshot
  serviceVisibleEpoch: number | null
}): ResultFreshness {
  if (!input.hasResults) return "current"
  if (input.lifecycle.state !== "ready" || input.resultGeneration !== input.lifecycle.generation) {
    return "interrupted"
  }
  if (
    input.current === "current"
    && input.resultVisibleEpoch !== null
    && input.serviceVisibleEpoch !== null
    && input.resultVisibleEpoch !== input.serviceVisibleEpoch
  ) {
    return "stale"
  }
  return input.current
}

export function detailReadMayContinue(
  lifecycle: DaemonLifecycleSnapshot,
  openedGeneration: number,
): boolean {
  return lifecycle.state === "ready" && lifecycle.generation === openedGeneration
}

export function isDaemonLifecycleSnapshot(value: unknown): value is DaemonLifecycleSnapshot {
  if (typeof value !== "object" || value === null) return false
  const candidate = value as Partial<DaemonLifecycleSnapshot>
  const state = String(candidate.state)
  const blockedReason = candidate.blocked_reason
  const lastExit = candidate.last_exit
  return candidate.schema_version === "resume-ir.desktop-daemon-lifecycle.v1"
    && ["starting", "ready", "recovering", "circuit_open", "blocked"].includes(state)
    && Number.isSafeInteger(candidate.generation)
    && Number(candidate.generation) >= 0
    && Number.isSafeInteger(candidate.restart_attempt)
    && Number(candidate.restart_attempt) >= 0
    && candidate.restart_budget === 5
    && Number(candidate.restart_attempt) <= candidate.restart_budget
    && (candidate.retry_delay_ms === null || (Number.isSafeInteger(candidate.retry_delay_ms) && Number(candidate.retry_delay_ms) >= 0 && Number(candidate.retry_delay_ms) <= 300_000))
    && Number.isSafeInteger(candidate.consecutive_heartbeat_failures)
    && Number(candidate.consecutive_heartbeat_failures) >= 0
    && Number(candidate.consecutive_heartbeat_failures) <= 3
    && (blockedReason === null || ["configuration_invalid", "runtime_integrity", "protocol_mismatch", "ownership_conflict", "supervisor_unavailable"].includes(String(blockedReason)))
    && (lastExit === null || ["child_exited", "startup_timeout", "heartbeat_timeout", "start_failed", "control_plane_failure"].includes(String(lastExit)))
    && (state === "blocked") === (blockedReason !== null)
}
