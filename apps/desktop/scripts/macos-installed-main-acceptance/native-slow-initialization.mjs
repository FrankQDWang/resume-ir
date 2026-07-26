import { toolSucceeded } from "./bounded-process.mjs";
import { POLL_MS, TOOL_TIMEOUT_MS, fail } from "./core.mjs";

export const SLOW_RUNTIME_STOP_MS = 11_000;
export const SLOW_MONITOR_TIMEOUT_MS = 30_000;

function exactExecutableCommand(command, executable) {
  return command === executable || command.startsWith(`${executable} `);
}

function parseProcessTable(source) {
  const processes = [];
  for (const line of source.split("\n")) {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.+)$/);
    if (!match) continue;
    processes.push({
      pid: Number(match[1]),
      ppid: Number(match[2]),
      pgid: Number(match[3]),
      command: match[4],
    });
  }
  return processes;
}

export async function defaultProcessTable(runTool) {
  const result = await runTool(
    "/bin/ps",
    ["-ww", "-axo", "pid=,ppid=,pgid=,command="],
    { timeoutMs: TOOL_TIMEOUT_MS },
  );
  if (!toolSucceeded(result) || result.stderr !== "") {
    fail("slow_initialization_monitor_failed");
  }
  return parseProcessTable(result.stdout);
}

function cancellation() {
  let cancel;
  const promise = new Promise((resolve) => {
    cancel = resolve;
  });
  return { cancel, promise };
}

async function waitOrCancel(milliseconds, record, delay) {
  return Promise.race([
    delay(milliseconds).then(() => "elapsed"),
    record.cancellation.promise.then(() => "cancelled"),
  ]);
}

export function createSlowInitializationController({
  cleanupRunTool,
  delay,
  embeddingExecutable,
  expectedExecutables,
  killProcess,
  listProcesses,
  monitorTimeoutMs,
  now,
  readStartTime,
  runTool,
  slowStopMs,
}) {
  async function requireNoInstalledRuntime() {
    const processes = await listProcesses(runTool);
    if (
      processes.some(({ command }) =>
        Object.values(expectedExecutables).some((executable) =>
          exactExecutableCommand(command, executable),
        ),
      )
    ) {
      fail("installed_runtime_already_running");
    }
  }

  async function currentProcess(
    record,
    allowMissing = false,
    processRunTool = cleanupRunTool,
  ) {
    const processes = await listProcesses(processRunTool);
    const candidates = processes.filter(({ command }) =>
      exactExecutableCommand(command, embeddingExecutable),
    );
    if (candidates.length === 0 && allowMissing) return null;
    if (candidates.length !== 1) fail("slow_initialization_process_invalid");
    const candidate = candidates[0];
    const parent = processes.find(({ pid }) => pid === candidate.ppid);
    const application = parent
      ? processes.find(({ pid }) => pid === parent.ppid)
      : null;
    if (
      !parent ||
      !exactExecutableCommand(parent.command, expectedExecutables.daemon) ||
      !parent.command.includes(`--data-dir ${record.dataDir}`) ||
      !application ||
      !exactExecutableCommand(
        application.command,
        expectedExecutables.desktop,
      ) ||
      !application.command.includes(
        "--resume-ir-acceptance-session-authority=",
      )
    ) {
      fail("slow_initialization_process_invalid");
    }
    const startTime = await readStartTime(candidate.pid, cleanupRunTool);
    if (
      record.process &&
      (record.process.pid !== candidate.pid ||
        record.process.command !== candidate.command ||
        record.process.startTime !== startTime)
    ) {
      fail("slow_initialization_process_reused");
    }
    return Object.freeze({ ...candidate, startTime });
  }

  async function continueStopped(record) {
    if (!record.stopped) return;
    const observed = await currentProcess(record);
    if (observed.pid !== record.process.pid) {
      fail("slow_initialization_process_reused");
    }
    try {
      killProcess(record.process.pid, "SIGCONT");
    } catch {
      fail("slow_initialization_continue_failed");
    }
    record.stopped = false;
  }

  async function runMonitor(record) {
    const deadline = now() + monitorTimeoutMs;
    while (!record.cancelled && now() < deadline) {
      const observed = await currentProcess(record, true, runTool);
      if (!observed) {
        if ((await waitOrCancel(POLL_MS, record, delay)) === "cancelled") {
          return "cancelled";
        }
        continue;
      }
      record.process = observed;
      const confirmed = await currentProcess(record, false, runTool);
      if (
        confirmed.pid !== observed.pid ||
        confirmed.startTime !== observed.startTime ||
        confirmed.command !== observed.command
      ) {
        fail("slow_initialization_process_reused");
      }
      try {
        killProcess(observed.pid, "SIGSTOP");
      } catch {
        fail("slow_initialization_stop_failed");
      }
      record.stopped = true;
      const outcome = await waitOrCancel(slowStopMs, record, delay);
      await continueStopped(record);
      if (outcome === "cancelled") return "cancelled";
      return "completed";
    }
    if (record.cancelled) return "cancelled";
    fail("slow_initialization_monitor_timeout");
  }

  function prepare(dataDir) {
    return {
      cancellation: cancellation(),
      cancelled: false,
      dataDir,
      monitorError: null,
      monitorOutcome: null,
      monitorTask: null,
      process: null,
      stopped: false,
    };
  }

  function activate(record) {
    record.monitorTask = runMonitor(record).then(
      (outcome) => {
        record.monitorOutcome = outcome;
        return outcome;
      },
      (error) => {
        record.monitorOutcome = "failed";
        record.monitorError = error;
        return "failed";
      },
    );
  }

  function requireCompleted(record, required) {
    if (required && record.monitorOutcome !== "completed") {
      if (record.monitorError) throw record.monitorError;
      fail("installed_fault_not_completed");
    }
  }

  async function restore(record, required) {
    await continueStopped(record);
    record.cancelled = true;
    record.cancellation.cancel();
    await record.monitorTask;
    requireCompleted(record, required);
  }

  return Object.freeze({
    activate,
    prepare,
    requireCompleted,
    requireNoInstalledRuntime,
    restore,
  });
}
