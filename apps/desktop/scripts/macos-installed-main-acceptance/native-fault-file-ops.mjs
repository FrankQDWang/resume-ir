import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, realpath, rename } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { toolSucceeded } from "./bounded-process.mjs";
import { TOOL_TIMEOUT_MS, fail } from "./core.mjs";

export const BACKUP_SUFFIX = ".resume-ir-installed-acceptance-backup-v1";

const SHA256 = /^[a-f0-9]{64}$/;
const ATOMIC_RENAME_EXCLUSIVE = fileURLToPath(
  new URL("./atomic-rename-exclusive.rb", import.meta.url),
);

export function permissionFailure(error) {
  return error?.code === "EACCES" || error?.code === "EPERM";
}

export function missingObject(error) {
  return error?.code === "ENOENT";
}

function objectIdentity(metadata) {
  return Object.freeze({
    dev: metadata.dev,
    ino: metadata.ino,
    mode: metadata.mode,
    size: metadata.size,
    uid: metadata.uid,
  });
}

export function sameIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.uid === right.uid
  );
}

export function fileSha256(file) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const input = createReadStream(file);
    input.on("error", reject);
    input.on("data", (chunk) => hash.update(chunk));
    input.on("end", () => resolve(hash.digest("hex")));
  });
}

export async function optionalMetadata(file) {
  try {
    return await lstat(file);
  } catch (error) {
    if (missingObject(error)) return null;
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail("installed_fault_target_invalid");
  }
}

async function requireSafeParent(file, uid) {
  const parent = path.dirname(file);
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([lstat(parent), realpath(parent)]);
  } catch (error) {
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail("installed_fault_target_invalid");
  }
  if (metadata.uid !== uid) fail("installed_fault_permission_denied");
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    resolved !== parent ||
    (metadata.mode & 0o022) !== 0
  ) {
    fail("installed_fault_target_unsafe");
  }
}

export async function requireSafeFile(file, { executable, uid }) {
  await requireSafeParent(file, uid);
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([lstat(file), realpath(file)]);
  } catch (error) {
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail("installed_fault_target_invalid");
  }
  if (metadata.uid !== uid) fail("installed_fault_permission_denied");
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    resolved !== file ||
    metadata.size < 1 ||
    (metadata.mode & 0o022) !== 0 ||
    (metadata.mode & 0o400) === 0 ||
    (executable && (metadata.mode & 0o100) === 0)
  ) {
    fail("installed_fault_target_unsafe");
  }
  return objectIdentity(metadata);
}

export async function atomicRename(source, destination, code) {
  try {
    await rename(source, destination);
  } catch (error) {
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail(code);
  }
}

export async function systemAtomicRename(runTool, source, destination, code) {
  const result = await runTool(
    "/usr/bin/ruby",
    [ATOMIC_RENAME_EXCLUSIVE, source, destination],
    { timeoutMs: TOOL_TIMEOUT_MS },
  );
  if (toolSucceeded(result) && result.stdout === "" && result.stderr === "") {
    return;
  }
  if (result?.status === 73) fail("installed_fault_permission_denied");
  if (result?.status === 74) fail("installed_fault_backup_conflict");
  fail(code);
}

export function backupPath(target, digest) {
  if (!SHA256.test(digest ?? "")) fail("installed_fault_backup_invalid");
  return path.join(
    path.dirname(target),
    `.${path.basename(target)}.${digest}${BACKUP_SUFFIX}`,
  );
}

export function backupDigest(target, candidate) {
  const prefix = `.${path.basename(target)}.`;
  if (!candidate.startsWith(prefix) || !candidate.endsWith(BACKUP_SUFFIX)) {
    return null;
  }
  const digest = candidate.slice(
    prefix.length,
    candidate.length - BACKUP_SUFFIX.length,
  );
  return SHA256.test(digest) ? digest : null;
}
