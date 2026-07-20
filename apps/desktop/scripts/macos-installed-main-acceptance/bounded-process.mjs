import { spawn } from "node:child_process";

import {
  CLONE_TIMEOUT_MS,
  MAX_TOOL_OUTPUT_BYTES,
  TOOL_TIMEOUT_MS,
  createExitMonitor,
  fail,
  signalProcessGroup,
  throwIfAborted,
  validAbsolutePath,
  waitBounded,
} from "./core.mjs";

const MAX_CALLER_STDOUT_BYTES = 8 * 1024 * 1024;

export async function runBoundedTool(
  command,
  args,
  {
    timeoutMs = TOOL_TIMEOUT_MS,
    env = {},
    cwd = "/",
    killProcess = process.kill.bind(process),
    maxStdoutBytes = MAX_TOOL_OUTPUT_BYTES,
    onSettled,
    onSpawn,
    signal,
    spawnTool = spawn,
    stdoutMode = "text",
  } = {},
) {
  if (
    !validAbsolutePath(command) ||
    !Array.isArray(args) ||
    !args.every((arg) => typeof arg === "string" && !arg.includes("\0")) ||
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 25 ||
    timeoutMs > CLONE_TIMEOUT_MS ||
    !Number.isSafeInteger(maxStdoutBytes) ||
    maxStdoutBytes < 1 ||
    maxStdoutBytes > MAX_CALLER_STDOUT_BYTES ||
    (onSpawn !== undefined && typeof onSpawn !== "function") ||
    (onSettled !== undefined && typeof onSettled !== "function") ||
    !["text", "buffer"].includes(stdoutMode)
  ) {
    fail("tool_invocation_invalid");
  }
  throwIfAborted(signal);
  let child;
  try {
    child = spawnTool(command, args, {
      cwd,
      detached: true,
      env,
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
  } catch {
    fail("tool_unavailable");
  }
  const monitor = createExitMonitor(child);
  let stdout = Buffer.alloc(0);
  let stderr = Buffer.alloc(0);
  let overflow = false;
  const collect = (current, chunk, limit) => {
    if (current.length + chunk.length > limit) {
      overflow = true;
      signalProcessGroup(child, "SIGKILL", killProcess);
      return current;
    }
    return Buffer.concat([current, chunk], current.length + chunk.length);
  };
  child.stdout?.on("data", (chunk) => {
    stdout = collect(stdout, chunk, maxStdoutBytes);
  });
  child.stderr?.on("data", (chunk) => {
    stderr = collect(stderr, chunk, MAX_TOOL_OUTPUT_BYTES);
  });
  child.stdout?.on("error", () => {});
  child.stderr?.on("error", () => {});
  let timer;
  let abortListener;
  const abort = new Promise((resolve) => {
    if (!signal) return;
    abortListener = () => resolve("aborted");
    if (signal.aborted) resolve("aborted");
    else signal.addEventListener("abort", abortListener, { once: true });
  });
  const terminate = async () => {
    if (monitor.settled) return true;
    signalProcessGroup(child, "SIGTERM", killProcess);
    if (await waitBounded(monitor.promise, 1_000)) return true;
    signalProcessGroup(child, "SIGKILL", killProcess);
    return waitBounded(monitor.promise, 1_000);
  };
  try {
    try {
      await onSpawn?.(child);
    } catch (error) {
      if (!(await terminate())) fail("cleanup_failed");
      throw error;
    }
    const outcome = await Promise.race([
      monitor.promise.then(() => "completed"),
      abort,
      new Promise((resolve) => {
        timer = setTimeout(() => resolve("timed_out"), timeoutMs);
      }),
    ]);
    if (outcome === "aborted") {
      if (!(await terminate())) fail("cleanup_failed");
      fail("acceptance_interrupted");
    }
    if (outcome === "timed_out") {
      if (!(await terminate())) fail("cleanup_failed");
    }
    return {
      status: outcome === "completed" && !overflow ? child.exitCode : null,
      stdout: stdoutMode === "buffer" ? stdout : stdout.toString("utf8"),
      stderr: stderr.toString("utf8"),
      timedOut: outcome === "timed_out",
      overflow,
    };
  } finally {
    clearTimeout(timer);
    if (signal && abortListener) {
      signal.removeEventListener("abort", abortListener);
    }
    await onSettled?.(child, { settled: monitor.settled });
  }
}

export function toolSucceeded(result) {
  return (
    result?.status === 0 &&
    result.timedOut === false &&
    result.overflow === false
  );
}
