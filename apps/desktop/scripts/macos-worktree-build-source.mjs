import { constants } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import path from "node:path";

import {
  captureSourceIdentity,
  verifyImmutableSnapshotSource,
} from "./macos-source-identity.mjs";

function snapshotError() {
  return new Error("macOS worktree build snapshot is invalid");
}

async function requirePrivateDirectory(directory) {
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
    await chmod(directory, 0o700);
    const [metadata, resolved] = await Promise.all([
      lstat(directory),
      realpath(directory),
    ]);
    if (
      !metadata.isDirectory() ||
      metadata.isSymbolicLink() ||
      resolved !== directory ||
      (metadata.mode & 0o777) !== 0o700
    ) {
      throw snapshotError();
    }
  } catch (error) {
    if (error?.message === "macOS worktree build snapshot is invalid") throw error;
    throw snapshotError();
  }
}

async function copySourceRecord(sourceRoot, destinationRoot, record) {
  const source = path.join(sourceRoot, ...record.relative.split("/"));
  const destination = path.join(
    destinationRoot,
    ...record.relative.split("/"),
  );
  await mkdir(path.dirname(destination), { recursive: true, mode: 0o700 });
  await copyFile(
    source,
    destination,
    constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE,
  );
  await chmod(destination, record.executable ? 0o755 : 0o644);
}

async function existingSnapshot(directory, identity) {
  try {
    await verifyImmutableSnapshotSource({
      repoRoot: directory,
      expected: identity,
    });
    return true;
  } catch {
    return false;
  }
}

export async function createImmutableWorktreeSnapshot({
  repoRoot,
  cacheRoot = path.join(repoRoot, ".cache", "macos-worktree-build"),
}) {
  if (
    !path.isAbsolute(repoRoot ?? "") ||
    !path.isAbsolute(cacheRoot ?? "") ||
    cacheRoot === repoRoot
  ) {
    throw snapshotError();
  }
  const captured = await captureSourceIdentity({
    repoRoot,
    authority: "worktree_snapshot",
  });
  const sources = path.join(cacheRoot, "sources");
  await requirePrivateDirectory(sources);
  const directory = path.join(
    sources,
    `${captured.identity.base_commit}-${captured.identity.source_tree_sha256}`,
  );
  if (await existingSnapshot(directory, captured.identity)) {
    return Object.freeze({
      repoRoot: directory,
      source: captured.identity,
      reused: true,
    });
  }
  try {
    await lstat(directory);
    throw snapshotError();
  } catch (error) {
    if (error?.message === "macOS worktree build snapshot is invalid") throw error;
    if (error?.code !== "ENOENT") throw snapshotError();
  }

  let temporary;
  try {
    temporary = await mkdtemp(path.join(sources, ".incoming-"));
    await chmod(temporary, 0o700);
    for (const record of captured.records) {
      await copySourceRecord(repoRoot, temporary, record);
    }
    await verifyImmutableSnapshotSource({
      repoRoot: temporary,
      expected: captured.identity,
    });
    try {
      await rename(temporary, directory);
      temporary = undefined;
    } catch (error) {
      if (!(await existingSnapshot(directory, captured.identity))) throw error;
    }
    await verifyImmutableSnapshotSource({
      repoRoot: directory,
      expected: captured.identity,
    });
  } catch (error) {
    if (error?.message === "macOS worktree build snapshot is invalid") throw error;
    throw snapshotError();
  } finally {
    if (temporary) await rm(temporary, { recursive: true, force: true }).catch(() => {});
  }
  return Object.freeze({
    repoRoot: directory,
    source: captured.identity,
    reused: false,
  });
}
