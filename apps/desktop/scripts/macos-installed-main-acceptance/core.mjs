import path from "node:path";
import { fileURLToPath } from "node:url";

export const ACCEPTANCE_SCHEMA = "resume-ir.macos-installed-main-acceptance.v1";
export const INSTALLED_APP_BUNDLE = "/Applications/resume-ir.app";
export const TARGET_TRIPLE = "aarch64-apple-darwin";
export const APP_DATA_DIRECTORY = "local.resume-ir.desktop";
export const APP_EXECUTABLE = path.join(
  INSTALLED_APP_BUNDLE,
  "Contents",
  "MacOS",
  "resume-desktop",
);
export const DAEMON_EXECUTABLE = path.join(
  INSTALLED_APP_BUNDLE,
  "Contents",
  "MacOS",
  "resume-daemon",
);
export const EMBEDDING_EXECUTABLE = path.join(
  INSTALLED_APP_BUNDLE,
  "Contents",
  "MacOS",
  "resume-embedding-runtime",
);
export const PDF_RENDER_EXECUTABLE = path.join(
  INSTALLED_APP_BUNDLE,
  "Contents",
  "MacOS",
  "resume-pdf-render-runtime",
);
export const DEFAULT_INSTALLED_EXECUTABLES = Object.freeze({
  desktop: APP_EXECUTABLE,
  daemon: DAEMON_EXECUTABLE,
  embedding_runtime: EMBEDDING_EXECUTABLE,
  pdf_renderer: PDF_RENDER_EXECUTABLE,
});
export const ENTRY_SCRIPT_FILE = fileURLToPath(
  new URL("../macos-installed-main-acceptance.mjs", import.meta.url),
);
export const REPO_ROOT = fileURLToPath(
  new URL("../../../../", import.meta.url),
);
export const SOURCE_PACKAGE_FILE = path.join(
  REPO_ROOT,
  "apps",
  "desktop",
  "package.json",
);
export const SOURCE_TAURI_CONFIG_FILE = path.join(
  REPO_ROOT,
  "apps",
  "desktop",
  "src-tauri",
  "tauri.conf.json",
);
export const SOURCE_ICON_FILE = path.join(
  REPO_ROOT,
  "apps",
  "desktop",
  "src-tauri",
  "icons",
  "icon.icns",
);
export const WORKSPACE_PREFIX = "resume-ir-installed-main-";
export const WORKSPACE_MARKER = ".resume-ir-installed-main-acceptance.v1";
export const WORKSPACE_MARKER_SCHEMA =
  "resume-ir.macos-installed-main-workspace.v1";
export const ENDPOINT_FILE = "ipc.endpoints.json";
export const AUTH_FILE = "ipc.auth";
export const ACTIVE_STORE_MANIFEST = "metadata-active.v1";
export const DATA_OWNER_LOCK = "data-directory-owner.lock";
export const LIFECYCLE_RECEIPT = "desktop-daemon-lifecycle.v1.json";
export const LOCK_READY = Buffer.from(
  "resume-ir.installed-main-publication-lock.ready.v1\n",
  "utf8",
);
export const COW_CLONE_COMPLETE = "resume-ir.apfs-cow-clone.complete.v1\n";
export const MAX_TOOL_OUTPUT_BYTES = 2 * 1024 * 1024;
export const MAX_HTTP_BYTES = 2 * 1024 * 1024;
export const MAX_OWNER_FILE_BYTES = 16 * 1024;
export const MAX_DIAGNOSTICS_BYTES = 2 * 1024 * 1024;
export const TOOL_TIMEOUT_MS = 15_000;
export const CLONE_TIMEOUT_MS = 20 * 60_000;
export const COLD_READY_TIMEOUT_MS = 20 * 60_000;
export const READY_TIMEOUT_MS = 2 * 60_000;
export const CONTENTION_TIMEOUT_MS = 2 * 60_000;
export const CONTENTION_CONVERGENCE_TIMEOUT_MS = 5 * 60_000;
export const PERSISTENT_CONTENTION_TIMEOUT_MS = 3 * 60_000;
export const QUIT_TIMEOUT_MS = 15_000;
export const HTTP_TIMEOUT_MS = 2_500;
export const POLL_MS = 200;
export const DIGEST = /^[a-f0-9]{64}$/;
export const GIT_HEAD = /^[a-f0-9]{40}$/;
export const RUN_ID = /^[a-f0-9]{64}$/;
export const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
export const CONTENTION_LOCKS = Object.freeze({
  fulltext: ["search-index", "snapshot-publication.lock"],
  vector: ["vector-index", "snapshot-publication.lock"],
});
export const CONTENTION_ERROR_KINDS = Object.freeze({
  fulltext: "fulltext_publication_busy",
  vector: "vector_publication_busy",
});

export class AcceptanceError extends Error {
  constructor(code) {
    super(code);
    this.name = "AcceptanceError";
    this.code = code;
  }
}

export function fail(code) {
  throw new AcceptanceError(code);
}

export function asAcceptanceError(error) {
  return error instanceof AcceptanceError
    ? error
    : new AcceptanceError("acceptance_internal_failure");
}

export function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) ===
      JSON.stringify([...expected].sort())
  );
}

export function currentUid() {
  const uid = process.getuid?.();
  if (!Number.isSafeInteger(uid) || uid < 0) fail("owner_identity_unavailable");
  return uid;
}

export function validAbsolutePath(value) {
  return (
    typeof value === "string" &&
    path.isAbsolute(value) &&
    value.length <= 4_096 &&
    !value.includes("\0")
  );
}

export function wait(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function throwIfAborted(signal) {
  if (signal?.aborted) fail("acceptance_interrupted");
}

export function createExitMonitor(child) {
  const monitor = { settled: false, outcome: undefined };
  monitor.promise = new Promise((resolve) => {
    const settle = (outcome) => {
      if (monitor.settled) return;
      monitor.settled = true;
      monitor.outcome = outcome;
      resolve(outcome);
    };
    child.once("error", () => settle("spawn_error"));
    child.once("exit", () => settle("exit"));
  });
  return monitor;
}

export async function waitBounded(promise, timeoutMs) {
  let timer;
  try {
    return await Promise.race([
      promise.then(() => true),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(false), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

export function signalProcessGroup(
  child,
  signal,
  killProcess = process.kill.bind(process),
) {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 0) return;
  try {
    killProcess(-child.pid, signal);
  } catch {
    try {
      child.kill(signal);
    } catch {
      // The exit monitor owns the final result.
    }
  }
}
