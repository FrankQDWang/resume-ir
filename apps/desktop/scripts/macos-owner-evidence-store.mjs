import { randomBytes } from "node:crypto";
import {
  chmod,
  link,
  lstat,
  mkdir,
  open,
  readFile,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const APP_DATA_DIRECTORY = "local.resume-ir.desktop";
const SAFE_FILE_NAME = /^[a-z0-9][a-z0-9._-]{0,127}$/;

function storeError(message) {
  return new Error(message);
}

function currentUid() {
  const uid = process.getuid?.();
  if (!Number.isSafeInteger(uid) || uid < 0) {
    throw storeError("application support root is invalid");
  }
  return uid;
}

async function requireSecureDirectory(directory, expectedBasename) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([lstat(directory), realpath(directory)]);
  } catch {
    throw storeError("application support root is invalid");
  }
  if (
    !path.isAbsolute(directory) ||
    path.resolve(directory) !== resolved ||
    path.basename(resolved) !== expectedBasename ||
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== currentUid() ||
    (metadata.mode & 0o022) !== 0
  ) {
    throw storeError("application support root is invalid");
  }
  return resolved;
}

async function resolveEvidenceDirectory(
  applicationSupportRoot,
  create,
  allowMissing = false,
  syncDirectory = defaultSyncDirectory,
) {
  const support = await requireSecureDirectory(
    applicationSupportRoot,
    "Application Support",
  );
  const directory = path.join(support, APP_DATA_DIRECTORY);
  if (create) {
    try {
      await mkdir(directory, { mode: 0o700 });
    } catch (error) {
      if (error?.code !== "EEXIST") {
        throw storeError("owner evidence directory is unavailable");
      }
    }
  } else {
    try {
      await lstat(directory);
    } catch (error) {
      if (allowMissing && error?.code === "ENOENT") return undefined;
      throw storeError("owner evidence directory is unavailable");
    }
  }
  try {
    const resolved = await requireSecureDirectory(
      directory,
      APP_DATA_DIRECTORY,
    );
    if (create) {
      await syncDirectory(resolved);
      await syncDirectory(support);
    }
    return resolved;
  } catch {
    throw storeError("owner evidence directory is invalid");
  }
}

function requireFileName(fileName) {
  if (!SAFE_FILE_NAME.test(fileName ?? "")) {
    throw storeError("owner evidence file name is invalid");
  }
}

async function defaultSyncDirectory(directory) {
  const handle = await open(directory, "r");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function readEvidenceFile({ file, maxBytes, validate, label, allowMissing }) {
  let metadata;
  let source;
  try {
    metadata = await lstat(file);
    if (
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      metadata.uid !== currentUid() ||
      (metadata.mode & 0o077) !== 0 ||
      metadata.size === 0 ||
      metadata.size > maxBytes
    ) {
      throw storeError(`${label} is invalid`);
    }
    source = await readFile(file, "utf8");
  } catch (error) {
    if (allowMissing && error?.code === "ENOENT") return undefined;
    if (error?.message === `${label} is invalid`) throw error;
    throw storeError(`${label} is unavailable`);
  }
  let value;
  try {
    value = validate(JSON.parse(source));
  } catch {
    throw storeError(`${label} is invalid`);
  }
  if (`${JSON.stringify(value)}\n` !== source) {
    throw storeError(`${label} is invalid`);
  }
  return { source, value };
}

async function writeTemporary(directory, fileName, source, label) {
  const temporary = path.join(
    directory,
    `.${fileName}.${randomBytes(8).toString("hex")}.tmp`,
  );
  let handle;
  try {
    handle = await open(temporary, "wx", 0o600);
    await handle.writeFile(source, "utf8");
    await handle.sync();
    await handle.close();
    handle = undefined;
    await chmod(temporary, 0o600);
    return temporary;
  } catch {
    await handle?.close().catch(() => {});
    await rm(temporary, { force: true }).catch(() => {});
    throw storeError(`${label} temporary file is unavailable`);
  }
}

async function restoreEvidence({
  directory,
  target,
  fileName,
  previous,
  renameEntry,
  removeEntry,
  syncDirectory,
  label,
}) {
  if (previous) {
    const rollback = await writeTemporary(
      directory,
      fileName,
      previous.source,
      label,
    );
    try {
      await renameEntry(rollback, target);
      await syncDirectory(directory);
    } finally {
      await rm(rollback, { force: true }).catch(() => {});
    }
  } else {
    await removeEntry(target, { force: true });
    await syncDirectory(directory);
  }
}

export async function defaultApplicationSupportRoot() {
  let home;
  try {
    home = await realpath(os.homedir());
  } catch {
    throw storeError("application support root is invalid");
  }
  const root = path.join(home, "Library", "Application Support");
  await requireSecureDirectory(root, "Application Support");
  return root;
}

export function ownerEvidencePath(applicationSupportRoot, fileName) {
  requireFileName(fileName);
  if (!path.isAbsolute(applicationSupportRoot)) {
    throw storeError("application support root is invalid");
  }
  return path.join(applicationSupportRoot, APP_DATA_DIRECTORY, fileName);
}

export async function prepareOwnerEvidenceDirectory(
  applicationSupportRoot,
  { syncDirectory = defaultSyncDirectory } = {},
) {
  return resolveEvidenceDirectory(
    applicationSupportRoot,
    true,
    false,
    syncDirectory,
  );
}

function validatedSource({ value, maxBytes, validate, label }) {
  const validated = validate(value);
  const source = `${JSON.stringify(validated)}\n`;
  if (Buffer.byteLength(source, "utf8") > maxBytes) {
    throw storeError(`${label} is invalid`);
  }
  return { source, validated };
}

export async function readOwnerEvidence({
  applicationSupportRoot,
  fileName,
  maxBytes,
  validate,
  label,
  allowMissing = false,
}) {
  requireFileName(fileName);
  const directory = await resolveEvidenceDirectory(
    applicationSupportRoot,
    false,
    allowMissing,
  );
  if (!directory) return undefined;
  return readEvidenceFile({
    file: path.join(directory, fileName),
    maxBytes,
    validate,
    label,
    allowMissing,
  });
}

export async function persistOwnerEvidence({
  applicationSupportRoot,
  fileName,
  value,
  maxBytes,
  validate,
  label,
  operations = {},
}) {
  requireFileName(fileName);
  const { source, validated } = validatedSource({
    value,
    maxBytes,
    validate,
    label,
  });
  const directory = await resolveEvidenceDirectory(applicationSupportRoot, true);
  const target = path.join(directory, fileName);
  const previous = await readEvidenceFile({
    file: target,
    maxBytes,
    validate,
    label,
    allowMissing: true,
  });
  const renameEntry = operations.rename ?? rename;
  const removeEntry = operations.rm ?? rm;
  const syncDirectory = operations.syncDirectory ?? defaultSyncDirectory;
  let temporary = await writeTemporary(directory, fileName, source, label);
  let committed = false;
  try {
    await renameEntry(temporary, target);
    committed = true;
    temporary = undefined;
    await syncDirectory(directory);
  } catch {
    if (committed) {
      try {
        await restoreEvidence({
          directory,
          target,
          fileName,
          previous,
          renameEntry,
          removeEntry,
          syncDirectory,
          label,
        });
      } catch {
        throw storeError(`${label} rollback failed`);
      }
    }
    throw storeError(`${label} could not be persisted`);
  } finally {
    if (temporary) await rm(temporary, { force: true }).catch(() => {});
  }
  return validated;
}

export async function createOwnerEvidence({
  applicationSupportRoot,
  fileName,
  value,
  maxBytes,
  validate,
  label,
  operations = {},
}) {
  requireFileName(fileName);
  const { source, validated } = validatedSource({
    value,
    maxBytes,
    validate,
    label,
  });
  const directory = await resolveEvidenceDirectory(applicationSupportRoot, true);
  const target = path.join(directory, fileName);
  const linkEntry = operations.link ?? link;
  const removeEntry = operations.rm ?? rm;
  const syncDirectory = operations.syncDirectory ?? defaultSyncDirectory;
  let temporary = await writeTemporary(directory, fileName, source, label);
  let linked = false;
  try {
    await linkEntry(temporary, target);
    linked = true;
    await removeEntry(temporary, { force: false });
    temporary = undefined;
    await syncDirectory(directory);
  } catch (error) {
    if (linked) {
      try {
        await removeEntry(target, { force: false });
        await syncDirectory(directory);
      } catch {
        throw storeError(`${label} create rollback failed`);
      }
    }
    if (error?.code === "EEXIST") {
      throw storeError(`${label} already exists`);
    }
    throw storeError(`${label} could not be created`);
  } finally {
    if (temporary) await rm(temporary, { force: true }).catch(() => {});
  }
  return validated;
}

export async function removeOwnerEvidence({
  applicationSupportRoot,
  fileName,
  expectedValue,
  maxBytes,
  validate,
  label,
  operations = {},
}) {
  requireFileName(fileName);
  const directory = await resolveEvidenceDirectory(applicationSupportRoot, false);
  const target = path.join(directory, fileName);
  const current = await readEvidenceFile({
    file: target,
    maxBytes,
    validate,
    label,
    allowMissing: false,
  });
  if (
    expectedValue !== undefined &&
    JSON.stringify(current.value) !== JSON.stringify(validate(expectedValue))
  ) {
    throw storeError(`${label} does not match expected transaction`);
  }
  const renameEntry = operations.rename ?? rename;
  const removeEntry = operations.rm ?? rm;
  const syncDirectory = operations.syncDirectory ?? defaultSyncDirectory;
  let removed = false;
  try {
    await removeEntry(target, { force: false });
    removed = true;
    await syncDirectory(directory);
  } catch {
    if (removed) {
      try {
        await restoreEvidence({
          directory,
          target,
          fileName,
          previous: current,
          renameEntry,
          removeEntry,
          syncDirectory,
          label,
        });
      } catch {
        throw storeError(`${label} rollback failed`);
      }
    }
    throw storeError(`${label} could not be removed`);
  }
  return current.value;
}
