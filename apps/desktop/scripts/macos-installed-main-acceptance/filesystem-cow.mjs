import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  statfs,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  readInstallReceipt,
  verifyInstallReceipt,
} from "../macos-install-receipt.mjs";
import { runBoundedTool, toolSucceeded } from "./bounded-process.mjs";
import {
  ACCEPTANCE_SCHEMA,
  ACTIVE_STORE_MANIFEST,
  APP_DATA_DIRECTORY,
  COW_CLONE_COMPLETE,
  CLONE_TIMEOUT_MS,
  DATA_OWNER_LOCK,
  DIGEST,
  ENTRY_SCRIPT_FILE,
  LOCK_READY,
  MAX_OWNER_FILE_BYTES,
  RUN_ID,
  TOOL_TIMEOUT_MS,
  WORKSPACE_MARKER,
  WORKSPACE_MARKER_SCHEMA,
  WORKSPACE_PREFIX,
  AcceptanceError,
  asAcceptanceError,
  createExitMonitor,
  currentUid,
  fail,
  signalProcessGroup,
  validAbsolutePath,
  wait,
  waitBounded,
} from "./core.mjs";
import {
  readProcessStartTime,
  validateDurableProcessRecord,
} from "./process-identity.mjs";

const CLONEFILE_HELPER = fileURLToPath(
  new URL("./clonefile-helper.rb", import.meta.url),
);
export const FLOCK_HOLDER_FILE = fileURLToPath(
  new URL("./flock-holder.rb", import.meta.url),
);
const APFS_F_TYPE = 0x1an;
const WORKSPACE_STATES = new Set([
  "clone_prepared",
  "clone_active",
  "clone_ready",
  "launch_intent",
  "launch_pending",
  "app_running",
  "app_stopped",
]);

export function createAcceptanceRunId() {
  return randomBytes(32).toString("hex");
}

function exactMarkerKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) ===
      JSON.stringify([...expected].sort())
  );
}

function validPid(value) {
  return Number.isSafeInteger(value) && value > 1;
}

export function validateWorkspaceMarker(value) {
  const helper = value?.helper;
  const guardian = value?.guardian;
  const authorityAnchor = value?.authority_anchor;
  const application = value?.application;
  if (
    !exactMarkerKeys(value, [
      "schema_version",
      "acceptance_schema",
      "run_id",
      "state",
      "session_authority",
      "helper",
      "guardian",
      "authority_anchor",
      "application",
    ]) ||
    value.schema_version !== WORKSPACE_MARKER_SCHEMA ||
    value.acceptance_schema !== ACCEPTANCE_SCHEMA ||
    !RUN_ID.test(value.run_id ?? "") ||
    !WORKSPACE_STATES.has(value.state) ||
    (helper !== null &&
      (!exactMarkerKeys(helper, [
        "kind",
        "pid",
        "pgid",
        "start_time",
        "executable",
        "session_authority",
        "lock_kind",
      ]) ||
        !validateDurableProcessRecord(helper) ||
        !["cow_clone", "publication_lock"].includes(helper.kind) ||
        !validPid(helper.pid) ||
        helper.pgid !== helper.pid ||
        (helper.kind === "cow_clone"
          ? helper.lock_kind !== null
          : !["fulltext", "vector"].includes(helper.lock_kind)))) ||
    (guardian !== null &&
      (!exactMarkerKeys(guardian, [
        "pid",
        "pgid",
        "start_time",
        "executable",
        "session_authority",
      ]) ||
        !validateDurableProcessRecord(guardian) ||
        guardian.executable !== process.execPath ||
        guardian.pgid !== guardian.pid)) ||
    (authorityAnchor !== null &&
      (!exactMarkerKeys(authorityAnchor, [
        "pid",
        "pgid",
        "start_time",
        "executable",
        "session_authority",
      ]) ||
        !validateDurableProcessRecord(authorityAnchor))) ||
    (application !== null &&
      (!exactMarkerKeys(application, [
        "pid",
        "pgid",
        "start_time",
        "executable",
        "session_authority",
      ]) ||
        !validateDurableProcessRecord(application))) ||
    (value.session_authority !== null &&
      !RUN_ID.test(value.session_authority ?? "")) ||
    (value.session_authority !== null &&
      !["launch_intent", "launch_pending", "app_running"].includes(
        value.state,
      )) ||
    (value.session_authority === null &&
      (guardian !== null || authorityAnchor !== null || application !== null)) ||
    (guardian !== null &&
      guardian.session_authority !== value.session_authority) ||
    (authorityAnchor !== null &&
      (authorityAnchor.session_authority !== value.session_authority ||
        authorityAnchor.executable !== process.execPath ||
        authorityAnchor.pgid !== guardian?.pgid)) ||
    (application !== null &&
      (application.session_authority !== value.session_authority ||
        application.pgid !== guardian?.pgid)) ||
    (value.state === "clone_active" && helper?.kind !== "cow_clone") ||
    (value.state === "launch_intent" &&
      (value.session_authority === null ||
        guardian !== null ||
        authorityAnchor !== null ||
        application !== null)) ||
    (value.state === "launch_pending" &&
      (guardian === null || authorityAnchor !== null || application !== null)) ||
    (value.state === "app_running" &&
      (guardian === null || authorityAnchor === null || application === null)) ||
    (value.state === "app_stopped" &&
      (value.session_authority !== null ||
        guardian !== null ||
        authorityAnchor !== null ||
        application !== null))
  ) {
    fail("workspace_marker_invalid");
  }
  return value;
}

export async function readWorkspaceMarker(root) {
  const marker = await readPrivateJson(path.join(root, WORKSPACE_MARKER), 2_048);
  return Object.freeze(validateWorkspaceMarker(marker.value));
}

export async function persistWorkspaceMarker(root, marker) {
  validateWorkspaceMarker(marker);
  await requireSecureDirectory(root, { privateMode: true });
  const target = path.join(root, WORKSPACE_MARKER);
  const temporary = path.join(
    root,
    `.${WORKSPACE_MARKER}.tmp-${randomBytes(16).toString("hex")}`,
  );
  let directoryHandle;
  let temporaryHandle;
  try {
    temporaryHandle = await open(temporary, "wx", 0o600);
    await temporaryHandle.writeFile(`${JSON.stringify(marker)}\n`, "utf8");
    await temporaryHandle.sync();
    await temporaryHandle.close();
    temporaryHandle = undefined;
    await rename(temporary, target);
    directoryHandle = await open(root, constants.O_RDONLY);
    await directoryHandle.sync();
    await requirePrivateFile(target, { maxBytes: 2_048 });
  } catch (error) {
    await temporaryHandle?.close().catch(() => {});
    await rm(temporary, { force: true }).catch(() => {});
    if (error instanceof AcceptanceError) throw error;
    fail("workspace_marker_invalid");
  } finally {
    await directoryHandle?.close().catch(() => {});
  }
}

export async function updateWorkspaceMarker(root, runId, changes) {
  const current = await readWorkspaceMarker(root);
  if (current.run_id !== runId) fail("workspace_marker_invalid");
  const next = Object.freeze(
    validateWorkspaceMarker({ ...current, ...changes, run_id: runId }),
  );
  await persistWorkspaceMarker(root, next);
  return next;
}

export function workspaceMarker(runId, overrides = {}) {
  return Object.freeze(
    validateWorkspaceMarker({
      schema_version: WORKSPACE_MARKER_SCHEMA,
      acceptance_schema: ACCEPTANCE_SCHEMA,
      run_id: runId,
      state: "clone_prepared",
      session_authority: null,
      helper: null,
      guardian: null,
      authority_anchor: null,
      application: null,
      ...overrides,
    }),
  );
}

export function pathsOverlap(left, right) {
  const leftToRight = path.relative(left, right);
  const rightToLeft = path.relative(right, left);
  const contains = (relative) =>
    relative === "" ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative));
  return contains(leftToRight) || contains(rightToLeft);
}

export async function requireSecureDirectory(
  directory,
  { privateMode = false, allowedUids = [currentUid()] } = {},
) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([
      lstat(directory),
      realpath(directory),
    ]);
  } catch {
    fail("directory_invalid");
  }
  if (
    !validAbsolutePath(directory) ||
    resolved !== path.resolve(directory) ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    !allowedUids.includes(metadata.uid) ||
    (metadata.mode & (privateMode ? 0o077 : 0o022)) !== 0
  ) {
    fail("directory_invalid");
  }
  return { metadata, resolved };
}

export async function requirePrivateFile(
  file,
  {
    afterVerifiedOpen,
    allowLegacyReadOnly = false,
    empty = false,
    maxBytes,
    read = false,
  } = {},
) {
  let metadata;
  let resolved;
  let handle;
  let source;
  try {
    [metadata, resolved] = await Promise.all([lstat(file), realpath(file)]);
    if (
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      resolved !== path.resolve(file) ||
      metadata.uid !== currentUid() ||
      metadata.nlink !== 1 ||
      (allowLegacyReadOnly
        ? (metadata.mode & 0o022) !== 0 || (metadata.mode & 0o400) === 0
        : (metadata.mode & 0o777) !== 0o600) ||
      (empty ? metadata.size !== 0 : metadata.size === 0) ||
      (maxBytes !== undefined && metadata.size > maxBytes)
    ) {
      fail("private_file_invalid");
    }
    handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    const opened = await handle.stat();
    if (opened.dev !== metadata.dev || opened.ino !== metadata.ino) {
      fail("private_file_invalid");
    }
    if (read) {
      if (afterVerifiedOpen !== undefined) {
        if (typeof afterVerifiedOpen !== "function") {
          fail("private_file_invalid");
        }
        await afterVerifiedOpen(file);
      }
      source = await handle.readFile("utf8");
      const [openedAfter, current] = await Promise.all([
        handle.stat(),
        lstat(file),
      ]);
      if (
        openedAfter.dev !== metadata.dev ||
        openedAfter.ino !== metadata.ino ||
        openedAfter.size !== metadata.size ||
        current.dev !== metadata.dev ||
        current.ino !== metadata.ino ||
        current.size !== metadata.size
      ) {
        fail("private_file_invalid");
      }
    }
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail("private_file_invalid");
  } finally {
    await handle?.close().catch(() => {});
  }
  return read ? { metadata, source } : metadata;
}

export async function readVerifiedPrivateText(
  file,
  maxBytes,
  { afterVerifiedOpen } = {},
) {
  const { source } = await requirePrivateFile(file, {
    afterVerifiedOpen,
    maxBytes,
    read: true,
  });
  return source;
}

export async function readActiveStoreManifest(
  dataDir,
  { allowLegacyReadOnly = false } = {},
) {
  const file = path.join(dataDir, ACTIVE_STORE_MANIFEST);
  const { source } = await requirePrivateFile(file, {
    allowLegacyReadOnly,
    maxBytes: 512,
    read: true,
  });
  const match = source.match(
    /^resume-ir\.metadata-active\.v1\nfile=(metadata-v(27|28|29)-[a-f0-9]{16}\.sqlite3)\nschema=(27|28|29)\ndigest=([a-f0-9]{64})\n$/,
  );
  if (!match || Number(match[2]) !== Number(match[3])) {
    fail("active_store_manifest_invalid");
  }
  const [, fileName, , schemaValue, digest] = match;
  const schema = Number(schemaValue);
  if (fileName !== `metadata-v${schema}-${digest.slice(0, 16)}.sqlite3`) {
    fail("active_store_manifest_invalid");
  }
  return Object.freeze({ fileName, schema, digest });
}

export async function readPrivateJson(file, maxBytes = MAX_OWNER_FILE_BYTES) {
  const { source } = await requirePrivateFile(file, {
    maxBytes,
    read: true,
  });
  let value;
  try {
    value = JSON.parse(source);
  } catch {
    fail("private_json_invalid");
  }
  return { source, value };
}

export async function requireApfsDirectory(
  directory,
  { statfsTool = statfs } = {},
) {
  let filesystem;
  try {
    filesystem = await statfsTool(directory, { bigint: true });
  } catch {
    fail("apfs_clone_unavailable");
  }
  if (filesystem?.type !== APFS_F_TYPE) {
    fail("apfs_clone_unavailable");
  }
}

export async function acquireExistingLock(
  lockFile,
  { spawnTool = spawn } = {},
) {
  const expected = await requirePrivateFile(lockFile, { empty: true });
  let child;
  try {
    child = spawnTool("/usr/bin/ruby", [FLOCK_HOLDER_FILE, lockFile], {
      cwd: "/",
      detached: true,
      env: {},
      shell: false,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
  } catch {
    fail("lock_unavailable");
  }
  const monitor = createExitMonitor(child);
  let stdout = Buffer.alloc(0);
  child.stdin?.on("error", () => {});
  child.stdout?.on("error", () => {});
  child.stderr?.on("error", () => {});
  child.stderr?.resume();
  const ready = new Promise((resolve) => {
    child.stdout.on("data", (chunk) => {
      if (stdout.length + chunk.length > LOCK_READY.length) {
        resolve(false);
        return;
      }
      stdout = Buffer.concat([stdout, chunk], stdout.length + chunk.length);
      if (!LOCK_READY.subarray(0, stdout.length).equals(stdout)) {
        resolve(false);
      } else if (stdout.length === LOCK_READY.length) {
        resolve(true);
      }
    });
    monitor.promise.then(() => resolve(false));
  });
  const acquired = await Promise.race([ready, wait(2_000).then(() => false)]);
  if (!acquired) {
    child.stdin?.end();
    signalProcessGroup(child, "SIGKILL");
    await waitBounded(monitor.promise, 500);
    fail("lock_unavailable");
  }
  try {
    const current = await requirePrivateFile(lockFile, { empty: true });
    if (current.dev !== expected.dev || current.ino !== expected.ino) {
      fail("lock_unavailable");
    }
  } catch {
    child.stdin?.end();
    signalProcessGroup(child, "SIGKILL");
    await waitBounded(monitor.promise, 500);
    fail("lock_unavailable");
  }
  return { child, monitor, released: false };
}

export async function releaseExistingLock(capability) {
  if (capability.released) return;
  capability.released = true;
  capability.child.stdin?.end();
  if (await waitBounded(capability.monitor.promise, 2_000)) return;
  signalProcessGroup(capability.child, "SIGKILL");
  if (!(await waitBounded(capability.monitor.promise, 1_000))) {
    fail("lock_release_failed");
  }
}

async function readSourceModeWitness(sourceDirectory) {
  const entries = [];
  for (const [relative, optional] of [
    [".", false],
    [ACTIVE_STORE_MANIFEST, false],
    [DATA_OWNER_LOCK, false],
    ["search-publication.lock", false],
    ["index-publication.lock", true],
  ]) {
    let metadata;
    try {
      metadata = await lstat(path.join(sourceDirectory, relative));
    } catch (error) {
      if (optional && error?.code === "ENOENT") continue;
      fail("authorized_source_invalid");
    }
    if (
      metadata.isSymbolicLink() ||
      metadata.uid !== currentUid() ||
      (metadata.mode & 0o022) !== 0 ||
      (relative === "." ? !metadata.isDirectory() : !metadata.isFile())
    ) {
      fail("authorized_source_invalid");
    }
    entries.push({
      relative,
      dev: metadata.dev,
      ino: metadata.ino,
      mode: metadata.mode & 0o777,
      size: metadata.size,
    });
  }
  return entries;
}

async function forceCloneFile(sourceFile, destinationFile, mode) {
  if (mode !== constants.COPYFILE_FICLONE_FORCE) {
    fail("apfs_clone_failed");
  }
  const result = await runBoundedTool(
    "/usr/bin/ruby",
    [CLONEFILE_HELPER, sourceFile, destinationFile],
    { timeoutMs: TOOL_TIMEOUT_MS },
  );
  if (!toolSucceeded(result) || result.stdout !== "" || result.stderr !== "") {
    fail("apfs_clone_failed");
  }
}

export async function cloneDirectoryTreeCow(
  sourceDirectory,
  destinationDirectory,
  { cloneFile = forceCloneFile } = {},
) {
  const source = await requireSecureDirectory(sourceDirectory);
  const destinationParent = await requireSecureDirectory(
    path.dirname(destinationDirectory),
    { privateMode: true },
  );
  if (
    !validAbsolutePath(destinationDirectory) ||
    destinationDirectory !==
      path.join(
        destinationParent.resolved,
        path.basename(destinationDirectory),
      ) ||
    source.metadata.dev !== destinationParent.metadata.dev
  ) {
    fail("apfs_clone_failed");
  }
  try {
    await lstat(destinationDirectory);
    fail("apfs_clone_failed");
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    if (error?.code !== "ENOENT") fail("apfs_clone_failed");
  }

  let clonedFiles = 0;
  const cloneDirectory = async (sourceRoot, destinationRoot) => {
    const sourceMetadata = await lstat(sourceRoot);
    if (
      !sourceMetadata.isDirectory() ||
      sourceMetadata.isSymbolicLink() ||
      sourceMetadata.uid !== currentUid() ||
      (sourceMetadata.mode & 0o022) !== 0
    ) {
      fail("apfs_clone_failed");
    }
    await mkdir(destinationRoot, { mode: sourceMetadata.mode & 0o777 });
    await chmod(destinationRoot, sourceMetadata.mode & 0o777);
    const entries = await readdir(sourceRoot, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      if (
        entry.name === "." ||
        entry.name === ".." ||
        entry.name.includes("\0")
      ) {
        fail("apfs_clone_failed");
      }
      const sourceChild = path.join(sourceRoot, entry.name);
      const destinationChild = path.join(destinationRoot, entry.name);
      const metadata = await lstat(sourceChild);
      if (
        metadata.isSymbolicLink() ||
        metadata.uid !== currentUid() ||
        (metadata.mode & 0o022) !== 0
      ) {
        fail("apfs_clone_failed");
      }
      if (metadata.isDirectory()) {
        await cloneDirectory(sourceChild, destinationChild);
        continue;
      }
      if (!metadata.isFile() || metadata.nlink !== 1) {
        fail("apfs_clone_failed");
      }
      try {
        await cloneFile(
          sourceChild,
          destinationChild,
          constants.COPYFILE_FICLONE_FORCE,
        );
        await chmod(destinationChild, metadata.mode & 0o777);
      } catch {
        fail("apfs_clone_failed");
      }
      const cloned = await lstat(destinationChild);
      if (
        !cloned.isFile() ||
        cloned.isSymbolicLink() ||
        cloned.uid !== currentUid() ||
        cloned.nlink !== 1 ||
        cloned.size !== metadata.size ||
        (cloned.mode & 0o777) !== (metadata.mode & 0o777)
      ) {
        fail("apfs_clone_failed");
      }
      clonedFiles += 1;
    }
  };

  await cloneDirectory(source.resolved, destinationDirectory);
  if (clonedFiles === 0) fail("apfs_clone_failed");
  return { clonedFiles };
}

export async function createCowCloneWorkspace({
  authorizedSourceDataDir,
  temporaryParent,
  expectedComposition,
  runId = createAcceptanceRunId(),
  runTool = runBoundedTool,
  acquireLock = acquireExistingLock,
  releaseLock = releaseExistingLock,
  requireApfs = requireApfsDirectory,
  readManifest = readActiveStoreManifest,
}) {
  const source = await requireSecureDirectory(authorizedSourceDataDir);
  const temporary = await requireSecureDirectory(temporaryParent);
  if (
    pathsOverlap(source.resolved, temporary.resolved) ||
    source.metadata.dev !== temporary.metadata.dev
  ) {
    fail("apfs_clone_unavailable");
  }
  await Promise.all([
    requireApfs(source.resolved),
    requireApfs(temporary.resolved),
  ]);
  const ownerLock = await acquireLock(
    path.join(source.resolved, DATA_OWNER_LOCK),
  );
  let root;
  try {
    const sourceManifest = await readManifest(source.resolved, {
      allowLegacyReadOnly: true,
    });
    if (sourceManifest.schema !== 28) fail("authorized_source_schema_invalid");
    const sourceWitness = await readSourceModeWitness(source.resolved);
    root = await mkdtemp(path.join(temporary.resolved, WORKSPACE_PREFIX));
    await chmod(root, 0o700);
    root = await realpath(root);
    await persistWorkspaceMarker(root, workspaceMarker(runId));
    const applicationSupportRoot = path.join(
      root,
      "Library",
      "Application Support",
    );
    await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
    await chmod(path.join(root, "Library"), 0o700);
    await chmod(applicationSupportRoot, 0o700);
    const dataDir = path.join(applicationSupportRoot, APP_DATA_DIRECTORY);
    const copied = await runTool(
      process.execPath,
      [ENTRY_SCRIPT_FILE, "--internal-cow-clone", source.resolved, dataDir],
      {
        timeoutMs: CLONE_TIMEOUT_MS,
        onSpawn: async (child) => {
          const startTime = await readProcessStartTime(child.pid, runTool);
          await persistWorkspaceMarker(
            root,
            workspaceMarker(runId, {
              state: "clone_active",
              helper: {
                kind: "cow_clone",
                pid: child.pid,
                pgid: child.pid,
                start_time: startTime,
                executable: process.execPath,
                session_authority: runId,
                lock_kind: null,
              },
            }),
          );
        },
        onSettled: async (_child, outcome) => {
          if (outcome.settled) {
            await persistWorkspaceMarker(root, workspaceMarker(runId));
          }
        },
      },
    );
    if (
      !toolSucceeded(copied) ||
      copied.stdout !== COW_CLONE_COMPLETE ||
      copied.stderr !== ""
    ) {
      fail("apfs_clone_failed");
    }
    await requireSecureDirectory(dataDir);
    const cloneManifest = await readManifest(dataDir, {
      allowLegacyReadOnly: true,
    });
    if (
      cloneManifest.schema !== sourceManifest.schema ||
      cloneManifest.fileName !== sourceManifest.fileName ||
      cloneManifest.digest !== sourceManifest.digest
    ) {
      fail("apfs_clone_invalid");
    }
    const receipt = await readInstallReceipt({ applicationSupportRoot });
    verifyInstallReceipt({ receipt, composition: expectedComposition });
    const sourceManifestAfter = await readManifest(source.resolved, {
      allowLegacyReadOnly: true,
    });
    if (
      sourceManifestAfter.schema !== sourceManifest.schema ||
      sourceManifestAfter.fileName !== sourceManifest.fileName ||
      sourceManifestAfter.digest !== sourceManifest.digest
    ) {
      fail("authorized_source_changed");
    }
    const sourceWitnessAfter = await readSourceModeWitness(source.resolved);
    if (JSON.stringify(sourceWitnessAfter) !== JSON.stringify(sourceWitness)) {
      fail("authorized_source_changed");
    }
    await persistWorkspaceMarker(
      root,
      workspaceMarker(runId, { state: "clone_ready" }),
    );
    return {
      applicationSupportRoot,
      dataDir,
      home: root,
      root,
      runId,
      sourceSchema: sourceManifest.schema,
    };
  } catch (error) {
    if (root) {
      let marker;
      try {
        marker = await readWorkspaceMarker(root);
      } catch {
        marker = undefined;
      }
      if (marker?.helper === null) {
        await safeRemoveWorkspace(root, temporary.resolved, runId).catch(
          () => {},
        );
      }
    }
    throw asAcceptanceError(error);
  } finally {
    await releaseLock(ownerLock);
  }
}

export async function listRecoverableWorkspaces(
  temporaryParent,
  currentRunId,
) {
  const temporary = await requireSecureDirectory(temporaryParent);
  if (!RUN_ID.test(currentRunId ?? "")) fail("workspace_marker_invalid");
  let entries;
  try {
    entries = await readdir(temporary.resolved, { withFileTypes: true });
  } catch {
    fail("stale_workspace_recovery_failed");
  }
  const workspaces = [];
  for (const entry of entries) {
    if (
      !entry.name.startsWith(WORKSPACE_PREFIX) ||
      !entry.isDirectory() ||
      entry.isSymbolicLink()
    ) {
      continue;
    }
    const root = path.join(temporary.resolved, entry.name);
    let marker;
    try {
      await requireSecureDirectory(root, { privateMode: true });
      marker = await readWorkspaceMarker(root);
    } catch {
      continue;
    }
    if (marker.run_id === currentRunId) continue;
    workspaces.push(
      Object.freeze({
        dataDir: path.join(
          root,
          "Library",
          "Application Support",
          APP_DATA_DIRECTORY,
        ),
        home: root,
        marker,
        root,
        runId: marker.run_id,
      }),
    );
  }
  return Object.freeze(workspaces);
}

export async function safeRemoveWorkspace(
  root,
  temporaryParent,
  expectedRunId,
  { afterVerifiedRoot } = {},
) {
  let temporary;
  let verifiedRoot;
  let marker;
  try {
    temporary = await requireSecureDirectory(temporaryParent, {
      privateMode: true,
    });
    verifiedRoot = await requireSecureDirectory(root, { privateMode: true });
    marker = await readWorkspaceMarker(verifiedRoot.resolved);
  } catch {
    fail("cleanup_failed");
  }
  const relative = path.relative(temporary.resolved, verifiedRoot.resolved);
  if (
    !RUN_ID.test(expectedRunId ?? "") ||
    marker.run_id !== expectedRunId ||
    path.basename(verifiedRoot.resolved).startsWith(WORKSPACE_PREFIX) ===
      false ||
    path.dirname(verifiedRoot.resolved) !== temporary.resolved ||
    relative !== path.basename(verifiedRoot.resolved) ||
    (afterVerifiedRoot !== undefined && typeof afterVerifiedRoot !== "function")
  ) {
    fail("cleanup_failed");
  }
  const quarantine = path.join(
    temporary.resolved,
    `.resume-ir-installed-main-quarantine-${randomBytes(32).toString("hex")}`,
  );
  try {
    await afterVerifiedRoot?.();
    await rename(verifiedRoot.resolved, quarantine);
    const [quarantined, parentAfter] = await Promise.all([
      lstat(quarantine),
      lstat(temporary.resolved),
    ]);
    if (
      quarantined.dev !== verifiedRoot.metadata.dev ||
      quarantined.ino !== verifiedRoot.metadata.ino ||
      !quarantined.isDirectory() ||
      quarantined.isSymbolicLink() ||
      parentAfter.dev !== temporary.metadata.dev ||
      parentAfter.ino !== temporary.metadata.ino ||
      !parentAfter.isDirectory() ||
      parentAfter.isSymbolicLink()
    ) {
      fail("cleanup_failed");
    }
    await rm(quarantine, { recursive: true, force: false });
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail("cleanup_failed");
  }
}
