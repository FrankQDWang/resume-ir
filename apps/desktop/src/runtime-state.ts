export type DaemonLifecycle = "starting" | "running" | "retry_wait" | "circuit_open" | "blocked"
export type DaemonService = "ready" | "degraded" | "repairing" | "initializing" | "blocked" | "unknown"
export type ResultFreshness = "current" | "stale" | "interrupted"

export type DaemonTransitionReason =
  | "initial_start"
  | "automatic_retry"
  | "manual_retry"
  | "control_plane_ready"
  | "child_exited"
  | "startup_timeout"
  | "heartbeat_timeout"
  | "start_failed"
  | "control_plane_failure"
  | "restart_budget_exhausted"
  | "half_open_retry"
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
  schema_version: "resume-ir.desktop-daemon-lifecycle.v2"
  state: DaemonLifecycle
  transition_reason: DaemonTransitionReason
  generation: number
  automatic_restart_attempt: number
  automatic_restart_limit: 5
  retry_after_ms: number | null
  heartbeat_failures: number
  last_exit: DaemonExitClass | null
}

export interface DaemonActionAuthority {
  epoch: number
  generation: number | null
  trusted: boolean
}

export interface DaemonActionAuthorityToken {
  epoch: number
  generation: number
}

export interface LifecycleReadability {
  epoch: number
  readable: boolean
}

export interface LifecycleReadabilityToken {
  epoch: number
}

export function initialLifecycleReadability(readable = false): LifecycleReadability {
  return { epoch: readable ? 1 : 0, readable }
}

export function observeReadableLifecycle(current: LifecycleReadability): LifecycleReadability {
  return { epoch: current.epoch + 1, readable: true }
}

export function invalidateLifecycleReadability(current: LifecycleReadability): LifecycleReadability {
  return { epoch: current.epoch + 1, readable: false }
}

export function captureLifecycleReadability(
  current: LifecycleReadability,
): LifecycleReadabilityToken | null {
  return current.readable ? { epoch: current.epoch } : null
}

export function lifecycleReadabilityIsCurrent(
  current: LifecycleReadability,
  token: LifecycleReadabilityToken,
): boolean {
  return current.readable && current.epoch === token.epoch
}

export function initialDaemonActionAuthority(generation: number | null = null): DaemonActionAuthority {
  return generation === null
    ? { epoch: 0, generation: null, trusted: false }
    : { epoch: 1, generation, trusted: true }
}

export function revokeDaemonActionAuthority(current: DaemonActionAuthority): DaemonActionAuthority {
  return { epoch: current.epoch + 1, generation: null, trusted: false }
}

export function trustDaemonActionAuthority(
  current: DaemonActionAuthority,
  generation: number,
): DaemonActionAuthority {
  if (current.trusted && current.generation === generation) return current
  return { epoch: current.epoch + 1, generation, trusted: true }
}

export function captureDaemonActionAuthority(
  current: DaemonActionAuthority,
  lifecycle: DaemonLifecycleSnapshot,
): DaemonActionAuthorityToken | null {
  if (!current.trusted || current.generation === null || !detailReadMayContinue(lifecycle, current.generation)) return null
  return { epoch: current.epoch, generation: current.generation }
}

export function daemonActionAuthorityIsCurrent(
  current: DaemonActionAuthority,
  lifecycle: DaemonLifecycleSnapshot,
  token: DaemonActionAuthorityToken,
): boolean {
  return current.trusted
    && current.epoch === token.epoch
    && current.generation === token.generation
    && detailReadMayContinue(lifecycle, token.generation)
}

export function blockedReasonMessage(reason: DaemonTransitionReason): string {
  switch (reason) {
    case "configuration_invalid": return "daemon 配置无效，需修正本地配置后重试"
    case "runtime_integrity": return "daemon 运行时完整性校验失败，已停止自动重启"
    case "protocol_mismatch": return "桌面端与 daemon 协议不匹配，需安装同一版本"
    case "ownership_conflict": return "已有其他进程持有 daemon 数据目录，已拒绝抢占"
    case "supervisor_unavailable": return "桌面原生监督器不可用，需重新启动应用"
    default: return "daemon 启动已阻止"
  }
}

export function lifecyclePollDelayMs(state: DaemonLifecycle): 1000 | 5000 {
  return state === "running" ? 5000 : 1000
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
  status: "ready" | "degraded" | "repairing" | "initializing" | "blocked" | "unknown"
}): DaemonService {
  if (input.httpStatus >= 200 && input.httpStatus < 300 && input.status === "ready") return "ready"
  if (input.status === "repairing") return "repairing"
  if (input.status === "initializing") return "initializing"
  if (input.status === "blocked") return "blocked"
  if (input.status === "unknown") return "unknown"
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
  if (input.lifecycle.state !== "running" || input.resultGeneration !== input.lifecycle.generation) {
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
  return lifecycle.state === "running" && lifecycle.generation === openedGeneration
}

export function statusAuthorityIsCurrent(
  lifecycle: DaemonLifecycleSnapshot,
  statusGeneration: number | null,
): boolean {
  return lifecycle.state === "running" && statusGeneration === lifecycle.generation
}

export function lifecycleInvalidatesStatusAuthority(
  previous: DaemonLifecycleSnapshot,
  next: DaemonLifecycleSnapshot,
  statusGeneration: number | null,
): boolean {
  return next.state !== "running"
    || next.generation !== previous.generation
    || statusGeneration !== next.generation
}

export function isDaemonLifecycleSnapshot(value: unknown): value is DaemonLifecycleSnapshot {
  if (typeof value !== "object" || value === null) return false
  const candidate = value as Partial<DaemonLifecycleSnapshot>
  const state = String(candidate.state)
  const transitionReason = candidate.transition_reason
  const lastExit = candidate.last_exit
  return hasExactKeys(candidate, [
    "schema_version", "state", "transition_reason", "generation",
    "automatic_restart_attempt", "automatic_restart_limit", "retry_after_ms",
    "heartbeat_failures", "last_exit",
  ])
    && candidate.schema_version === "resume-ir.desktop-daemon-lifecycle.v2"
    && ["starting", "running", "retry_wait", "circuit_open", "blocked"].includes(state)
    && ["initial_start", "automatic_retry", "manual_retry", "control_plane_ready", "child_exited", "startup_timeout", "heartbeat_timeout", "start_failed", "control_plane_failure", "restart_budget_exhausted", "half_open_retry", "configuration_invalid", "runtime_integrity", "protocol_mismatch", "ownership_conflict", "supervisor_unavailable"].includes(String(transitionReason))
    && Number.isSafeInteger(candidate.generation)
    && Number(candidate.generation) >= 0
    && Number.isSafeInteger(candidate.automatic_restart_attempt)
    && Number(candidate.automatic_restart_attempt) >= 0
    && candidate.automatic_restart_limit === 5
    && Number(candidate.automatic_restart_attempt) <= candidate.automatic_restart_limit
    && (candidate.retry_after_ms === null || (Number.isSafeInteger(candidate.retry_after_ms) && Number(candidate.retry_after_ms) >= 0 && Number(candidate.retry_after_ms) <= 300_000))
    && Number.isSafeInteger(candidate.heartbeat_failures)
    && Number(candidate.heartbeat_failures) >= 0
    && Number(candidate.heartbeat_failures) <= 3
    && (lastExit === null || ["child_exited", "startup_timeout", "heartbeat_timeout", "start_failed", "control_plane_failure"].includes(String(lastExit)))
    && lifecycleReasonMatches(state as DaemonLifecycle, String(transitionReason))
    && lifecycleFieldsMatch(candidate as DaemonLifecycleSnapshot)
}

function hasExactKeys(value: object, expected: string[]): boolean {
  const actual = Object.keys(value).sort()
  return actual.length === expected.length
    && actual.every((key, index) => key === [...expected].sort()[index])
}

function lifecycleReasonMatches(state: DaemonLifecycle, reason: string): boolean {
  if (state === "starting") return ["initial_start", "automatic_retry", "manual_retry", "half_open_retry"].includes(reason)
  if (state === "running") return reason === "control_plane_ready"
  if (state === "retry_wait") return ["child_exited", "startup_timeout", "heartbeat_timeout", "start_failed", "control_plane_failure"].includes(reason)
  if (state === "circuit_open") return reason === "restart_budget_exhausted"
  return ["configuration_invalid", "runtime_integrity", "protocol_mismatch", "ownership_conflict", "supervisor_unavailable"].includes(reason)
}

function lifecycleFieldsMatch(snapshot: DaemonLifecycleSnapshot): boolean {
  if (snapshot.state === "running") {
    return snapshot.generation > 0
      && snapshot.retry_after_ms === null
  }
  if (snapshot.state === "starting") {
    return snapshot.retry_after_ms === null
      && snapshot.heartbeat_failures === 0
      && (snapshot.transition_reason !== "initial_start" || snapshot.generation === 0)
  }
  if (snapshot.state === "retry_wait") {
    return snapshot.retry_after_ms !== null
      && snapshot.heartbeat_failures === 0
      && snapshot.last_exit === snapshot.transition_reason
  }
  if (snapshot.state === "circuit_open") {
    return snapshot.retry_after_ms !== null
      && snapshot.heartbeat_failures === 0
      && snapshot.automatic_restart_attempt === snapshot.automatic_restart_limit
  }
  return snapshot.retry_after_ms === null && snapshot.heartbeat_failures === 0
}
