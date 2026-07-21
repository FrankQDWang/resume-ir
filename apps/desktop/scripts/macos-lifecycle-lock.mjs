import { spawn } from "node:child_process";
import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const APP_DATA_DIRECTORY = "local.resume-ir.desktop";
const LOCK_TOOL = "/usr/bin/lockf";
const HOLDER_SCRIPT = fileURLToPath(
  new URL("./macos-lifecycle-lock-holder.mjs", import.meta.url),
);
const READY_BYTES = Buffer.from(
  "resume-ir.macos-lifecycle-lock.ready.v1\n",
  "utf8",
);
const MAX_STDERR_BYTES = 1_024;
const DEFAULT_STARTUP_TIMEOUT_MS = 2_000;
const DEFAULT_RELEASE_TIMEOUT_MS = 2_000;
const MAX_TIMEOUT_MS = 10_000;
const MIN_TIMEOUT_MS = 25;
const FORCED_EXIT_TIMEOUT_MS = 250;

export const LIFECYCLE_LOCK_FILE = "macos-lifecycle.lock";
export const INSTALLED_MAIN_ACCEPTANCE_LOCK_FILE =
  "macos-installed-main-acceptance.lock";

const LOCK_PURPOSE = Object.freeze({
  acceptance: INSTALLED_MAIN_ACCEPTANCE_LOCK_FILE,
  lifecycle: LIFECYCLE_LOCK_FILE,
});

const knownCapabilities = new WeakSet();
const activeCapabilities = new WeakSet();
const capabilityStates = new WeakMap();

function lifecycleLockError(kind) {
  const messages = {
    capability: "macOS lifecycle lock capability is invalid",
    file: "macOS lifecycle lock file is invalid",
    release: "macOS lifecycle lock release failed",
    unavailable: "macOS lifecycle lock is unavailable",
  };
  return new Error(messages[kind] ?? messages.unavailable);
}

function currentUid() {
  const uid = process.getuid?.();
  if (!Number.isSafeInteger(uid) || uid < 0) {
    throw lifecycleLockError("file");
  }
  return uid;
}

function boundedTimeout(value, fallback) {
  const candidate = value ?? fallback;
  if (
    !Number.isSafeInteger(candidate) ||
    candidate < MIN_TIMEOUT_MS ||
    candidate > MAX_TIMEOUT_MS
  ) {
    throw lifecycleLockError("unavailable");
  }
  return candidate;
}

async function requireSecureDirectory(directory, expectedBasename) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([
      lstat(directory),
      realpath(directory),
    ]);
  } catch {
    throw lifecycleLockError("file");
  }
  if (
    !path.isAbsolute(directory) ||
    path.resolve(directory) !== resolved ||
    path.basename(directory) !== expectedBasename ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== currentUid() ||
    (metadata.mode & 0o022) !== 0
  ) {
    throw lifecycleLockError("file");
  }
  return resolved;
}

async function requireSecureEvidenceDirectory(directory) {
  const support = await requireSecureDirectory(
    path.dirname(directory),
    "Application Support",
  );
  const resolved = await requireSecureDirectory(
    directory,
    APP_DATA_DIRECTORY,
  );
  if (resolved !== path.join(support, APP_DATA_DIRECTORY)) {
    throw lifecycleLockError("file");
  }
  return resolved;
}

async function syncDirectory(directory) {
  let handle;
  try {
    handle = await open(directory, "r");
    await handle.sync();
  } catch {
    throw lifecycleLockError("file");
  } finally {
    await handle?.close().catch(() => {});
  }
}

function isExpectedLockPath(lockFile, directory, purpose) {
  return (
    path.isAbsolute(lockFile) &&
    path.dirname(lockFile) === directory &&
    path.basename(lockFile) === LOCK_PURPOSE[purpose]
  );
}

function sameIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function requireSecureLockMetadata(metadata) {
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== currentUid() ||
    metadata.nlink !== 1 ||
    metadata.size !== 0 ||
    (metadata.mode & 0o777) !== 0o600
  ) {
    throw lifecycleLockError("file");
  }
}

async function validateLockFile(lockFile, purpose) {
  if (typeof lockFile !== "string" || !path.isAbsolute(lockFile)) {
    throw lifecycleLockError("file");
  }
  const directory = await requireSecureEvidenceDirectory(path.dirname(lockFile));
  if (
    !LOCK_PURPOSE[purpose] ||
    !isExpectedLockPath(lockFile, directory, purpose)
  ) {
    throw lifecycleLockError("file");
  }
  let metadata;
  let resolved;
  let handle;
  try {
    [metadata, resolved] = await Promise.all([lstat(lockFile), realpath(lockFile)]);
    requireSecureLockMetadata(metadata);
    if (resolved !== lockFile) throw lifecycleLockError("file");
    handle = await open(
      lockFile,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
    );
    const openedMetadata = await handle.stat();
    requireSecureLockMetadata(openedMetadata);
    if (!sameIdentity(metadata, openedMetadata)) {
      throw lifecycleLockError("file");
    }
  } catch (error) {
    if (error?.message === lifecycleLockError("file").message) throw error;
    throw lifecycleLockError("file");
  } finally {
    await handle?.close().catch(() => {});
  }
  return Object.freeze({ dev: metadata.dev, ino: metadata.ino });
}

async function createLockFile(lockFile) {
  let handle;
  let created = false;
  try {
    handle = await open(
      lockFile,
      constants.O_CREAT |
        constants.O_EXCL |
        constants.O_RDWR |
        (constants.O_NOFOLLOW ?? 0),
      0o600,
    );
    created = true;
    await handle.chmod(0o600);
    requireSecureLockMetadata(await handle.stat());
    await handle.sync();
  } catch (error) {
    if (error?.code !== "EEXIST") throw lifecycleLockError("file");
  } finally {
    await handle?.close().catch(() => {});
  }
  if (created) await syncDirectory(path.dirname(lockFile));
}

async function prepareLockFile({
  applicationSupportRoot,
  prepareEvidenceDirectory,
}, purpose) {
  if (typeof prepareEvidenceDirectory !== "function") {
    throw lifecycleLockError("file");
  }
  let directory;
  try {
    directory = await prepareEvidenceDirectory(applicationSupportRoot);
  } catch {
    throw lifecycleLockError("file");
  }
  directory = await requireSecureEvidenceDirectory(directory);
  const lockFile = path.join(directory, LOCK_PURPOSE[purpose]);
  await createLockFile(lockFile);
  await validateLockFile(lockFile, purpose);
  return lockFile;
}

export function prepareLifecycleLockFile(options) {
  return prepareLockFile(options, "lifecycle");
}

export function prepareInstalledMainAcceptanceLockFile(options) {
  return prepareLockFile(options, "acceptance");
}

function resolveRuntime(testRuntime) {
  if (testRuntime === undefined) {
    if (process.platform !== "darwin") {
      throw lifecycleLockError("unavailable");
    }
    return { holder: HOLDER_SCRIPT, node: process.execPath, tool: LOCK_TOOL };
  }
  if (
    !testRuntime ||
    Object.getPrototypeOf(testRuntime) !== Object.prototype ||
    !Object.keys(testRuntime).every((key) =>
      ["holderScript", "lockTool", "nodeExecutable", "platform"].includes(key),
    )
  ) {
    throw lifecycleLockError("unavailable");
  }
  const platform = testRuntime.platform;
  const tool = testRuntime.lockTool ?? LOCK_TOOL;
  const node = testRuntime.nodeExecutable ?? process.execPath;
  const holder = testRuntime.holderScript ?? HOLDER_SCRIPT;
  if (
    platform !== "darwin" ||
    !path.isAbsolute(tool) ||
    !path.isAbsolute(node) ||
    !path.isAbsolute(holder)
  ) {
    throw lifecycleLockError("unavailable");
  }
  return { holder, node, tool };
}

function createExitMonitor(child) {
  const monitor = { settled: false };
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

function endHolderInput(child) {
  if (child.stdin && !child.stdin.destroyed && !child.stdin.writableEnded) {
    child.stdin.end();
  }
}

function killChildGroup(child, signal) {
  if (Number.isSafeInteger(child.pid) && child.pid > 0) {
    try {
      process.kill(-child.pid, signal);
      return;
    } catch {
      // Fall through to the direct child when its process group is already gone.
    }
  }
  try {
    child.kill(signal);
  } catch {
    // Exit monitoring determines whether release actually completed.
  }
}

function unrefChild(child) {
  child.unref();
  child.stdin?.unref?.();
  child.stdout?.unref?.();
  child.stderr?.unref?.();
}

function refChild(child) {
  child.ref();
  child.stdin?.ref?.();
  child.stdout?.ref?.();
  child.stderr?.ref?.();
}

async function waitBounded(promise, timeoutMilliseconds) {
  let timer;
  try {
    return await Promise.race([
      promise.then(() => true),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(false), timeoutMilliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function terminateFailedAcquisition(child, monitor) {
  endHolderInput(child);
  if (await waitBounded(monitor.promise, FORCED_EXIT_TIMEOUT_MS)) return;
  killChildGroup(child, "SIGKILL");
  await waitBounded(monitor.promise, FORCED_EXIT_TIMEOUT_MS);
}

async function acquireLock({
  lockFile,
  startupTimeoutMs,
  releaseTimeoutMs,
  testRuntime,
}, purpose) {
  const startupTimeout = boundedTimeout(
    startupTimeoutMs,
    DEFAULT_STARTUP_TIMEOUT_MS,
  );
  const releaseTimeout = boundedTimeout(
    releaseTimeoutMs,
    DEFAULT_RELEASE_TIMEOUT_MS,
  );
  const initialIdentity = await validateLockFile(lockFile, purpose);
  const runtime = resolveRuntime(testRuntime);
  let child;
  try {
    child = spawn(
      runtime.tool,
      [
        "-k",
        "-n",
        "-s",
        "-t",
        "0",
        lockFile,
        runtime.node,
        runtime.holder,
      ],
      {
        detached: true,
        env: {},
        shell: false,
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true,
      },
    );
  } catch {
    throw lifecycleLockError("unavailable");
  }
  const monitor = createExitMonitor(child);
  child.stdin?.on("error", () => {});
  child.stdout?.on("error", () => {});
  child.stderr?.on("error", () => {});

  let ready = false;
  let readySource = Buffer.alloc(0);
  let stderrBytes = 0;
  let rejectStartup;
  const startup = new Promise((resolve, reject) => {
    rejectStartup = reject;
    child.stdout.on("data", (chunk) => {
      if (ready) {
        endHolderInput(child);
        killChildGroup(child, "SIGKILL");
        return;
      }
      if (chunk.length > READY_BYTES.length - readySource.length) {
        reject(lifecycleLockError("unavailable"));
        endHolderInput(child);
        killChildGroup(child, "SIGKILL");
        return;
      }
      readySource = Buffer.concat(
        [readySource, chunk],
        readySource.length + chunk.length,
      );
      if (
        !READY_BYTES.subarray(0, readySource.length).equals(readySource)
      ) {
        reject(lifecycleLockError("unavailable"));
        endHolderInput(child);
        killChildGroup(child, "SIGKILL");
        return;
      }
      if (readySource.length === READY_BYTES.length) {
        ready = true;
        resolve();
      }
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > MAX_STDERR_BYTES) {
        reject(lifecycleLockError("unavailable"));
        endHolderInput(child);
        killChildGroup(child, "SIGKILL");
      }
    });
    monitor.promise.then(() => {
      if (!ready) reject(lifecycleLockError("unavailable"));
    });
  });
  const startupTimer = setTimeout(
    () => rejectStartup(lifecycleLockError("unavailable")),
    startupTimeout,
  );
  try {
    await startup;
    const finalIdentity = await validateLockFile(lockFile, purpose);
    if (!sameIdentity(initialIdentity, finalIdentity) || monitor.settled) {
      throw lifecycleLockError("unavailable");
    }
  } catch (error) {
    await terminateFailedAcquisition(child, monitor);
    if (error?.message === lifecycleLockError("file").message) throw error;
    throw lifecycleLockError("unavailable");
  } finally {
    clearTimeout(startupTimer);
  }

  const capability = Object.freeze(Object.create(null));
  const state = {
    capability,
    child,
    monitor,
    purpose,
    releasePromise: undefined,
    releaseTimeout,
  };
  knownCapabilities.add(capability);
  activeCapabilities.add(capability);
  capabilityStates.set(capability, state);
  monitor.promise.then(() => activeCapabilities.delete(capability));
  unrefChild(child);
  return capability;
}

export function acquireLifecycleLock(options) {
  return acquireLock(options, "lifecycle");
}

export function acquireInstalledMainAcceptanceLock(options) {
  return acquireLock(options, "acceptance");
}

function requireLockCapability(capability, purpose) {
  const state = capabilityStates.get(capability);
  if (
    !knownCapabilities.has(capability) ||
    !activeCapabilities.has(capability) ||
    !state ||
    state.purpose !== purpose ||
    state.monitor.settled
  ) {
    throw lifecycleLockError("capability");
  }
  return capability;
}

export function requireLifecycleLockCapability(capability) {
  return requireLockCapability(capability, "lifecycle");
}

export function requireInstalledMainAcceptanceLockCapability(capability) {
  return requireLockCapability(capability, "acceptance");
}

async function releaseCapability(state) {
  activeCapabilities.delete(state.capability);
  refChild(state.child);
  endHolderInput(state.child);
  if (await waitBounded(state.monitor.promise, state.releaseTimeout)) return;
  killChildGroup(state.child, "SIGKILL");
  if (
    !(await waitBounded(state.monitor.promise, FORCED_EXIT_TIMEOUT_MS))
  ) {
    unrefChild(state.child);
    throw lifecycleLockError("release");
  }
}

function releaseLock(capability, purpose) {
  const state = capabilityStates.get(capability);
  if (!knownCapabilities.has(capability) || state?.purpose !== purpose) {
    return Promise.reject(lifecycleLockError("capability"));
  }
  state.releasePromise ??= releaseCapability(state);
  return state.releasePromise;
}

export function releaseLifecycleLock(capability) {
  return releaseLock(capability, "lifecycle");
}

export function releaseInstalledMainAcceptanceLock(capability) {
  return releaseLock(capability, "acceptance");
}
