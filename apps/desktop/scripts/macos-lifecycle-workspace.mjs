import { lstat, open, readdir, realpath, rename } from "node:fs/promises";
import path from "node:path";

const TRANSACTION_ID = /^[a-f0-9]{32}$/;
const RESERVED_ARTIFACT =
  /^\.resume-ir\.app\.(?:(?:install|reinstall|upgrade|uninstall)(?:\.|-)|lifecycle-trash\.)/;

function workspaceError(message) {
  return new Error(message);
}

function requireTransactionId(transactionId) {
  if (!TRANSACTION_ID.test(transactionId ?? "")) {
    throw workspaceError("lifecycle transaction id is invalid");
  }
  return transactionId;
}

function artifact(applicationsRoot, name) {
  if (!path.isAbsolute(applicationsRoot) || path.basename(applicationsRoot) !== "Applications") {
    throw workspaceError("Applications root is invalid");
  }
  return path.join(applicationsRoot, name);
}

export function lifecycleWorkspacePaths({
  applicationsRoot,
  operation,
  transactionId,
}) {
  const id = requireTransactionId(transactionId);
  if (
    !new Set(["install", "reinstall", "upgrade", "uninstall"]).has(
      operation,
    )
  ) {
    throw workspaceError("lifecycle operation is invalid");
  }
  const prefix = `.resume-ir.app.${operation}.${id}`;
  return {
    partial: artifact(applicationsRoot, `${prefix}.partial`),
    ready: artifact(applicationsRoot, `${prefix}.ready`),
    backup: artifact(applicationsRoot, `${prefix}.backup`),
    quarantine: artifact(applicationsRoot, `${prefix}.quarantine`),
    stageTombstone: artifact(
      applicationsRoot,
      `.resume-ir.app.lifecycle-trash.${id}.stage`,
    ),
    targetTombstone: artifact(
      applicationsRoot,
      `.resume-ir.app.lifecycle-trash.${id}.target`,
    ),
    backupTombstone: artifact(
      applicationsRoot,
      `.resume-ir.app.lifecycle-trash.${id}.backup`,
    ),
  };
}

export async function assertNoLifecycleArtifacts(applicationsRoot) {
  let entries;
  try {
    entries = await readdir(applicationsRoot);
  } catch {
    throw workspaceError("lifecycle workspace is unavailable");
  }
  if (entries.some((entry) => RESERVED_ARTIFACT.test(entry))) {
    throw workspaceError("orphan lifecycle workspace exists");
  }
}

async function syncDirectory(directory) {
  const handle = await open(directory, "r");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function syncTreeEntry(entry) {
  const metadata = await lstat(entry);
  if (metadata.isSymbolicLink()) {
    throw workspaceError("staged App durability tree is invalid");
  }
  if (metadata.isFile()) {
    const handle = await open(entry, "r");
    try {
      await handle.sync();
    } finally {
      await handle.close();
    }
    return;
  }
  if (!metadata.isDirectory()) {
    throw workspaceError("staged App durability tree is invalid");
  }
  const entries = (await readdir(entry)).sort();
  for (const child of entries) {
    await syncTreeEntry(path.join(entry, child));
  }
  await syncDirectory(entry);
}

export async function makeStagedAppDurable({
  appBundle,
  applicationsRoot,
}) {
  if (
    !path.isAbsolute(appBundle) ||
    path.dirname(appBundle) !== applicationsRoot
  ) {
    throw workspaceError("staged App durability arguments are invalid");
  }
  let resolved;
  try {
    resolved = await realpath(appBundle);
  } catch {
    throw workspaceError("staged App durability tree is invalid");
  }
  if (resolved !== appBundle) {
    throw workspaceError("staged App durability tree is invalid");
  }
  await syncTreeEntry(appBundle);
  await syncDirectory(applicationsRoot);
}

export async function publishDurableStage({
  partial,
  ready,
  applicationsRoot,
  move = rename,
}) {
  if (
    !path.isAbsolute(partial) ||
    !path.isAbsolute(ready) ||
    path.dirname(partial) !== applicationsRoot ||
    path.dirname(ready) !== applicationsRoot ||
    partial === ready
  ) {
    throw workspaceError("staged App publication arguments are invalid");
  }
  try {
    await move(partial, ready);
    await syncDirectory(applicationsRoot);
  } catch {
    throw workspaceError("staged App publication failed");
  }
}
