import { createHash } from "node:crypto";
import { lstat, open, readdir, realpath } from "node:fs/promises";
import path from "node:path";

import { MACOS_SYSTEM_TOOLS, runClosedSystemTool } from "./macos-system-tools.mjs";
import { verifyMainSourceProvenance } from "./macos-source-provenance.mjs";

export const SOURCE_AUTHORITIES = Object.freeze([
  "exact_main_commit",
  "worktree_snapshot",
]);

const SOURCE_COMMIT = /^[a-f0-9]{40}$/;
const DIGEST = /^[a-f0-9]{64}$/;
const MAX_FILES = 8_192;
const MAX_FILE_BYTES = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES = 512 * 1024 * 1024;
const MAX_GIT_OUTPUT_BYTES = 4 * 1024 * 1024;
const EXCLUDED_PARTS = new Set([".cache", ".git", "dist", "node_modules", "target"]);
const ROOT_FILES = new Set(["Cargo.lock", "Cargo.toml"]);
const ROOT_PREFIXES = ["apps/desktop/", "crates/"];
const GENERATED_DESKTOP_FILES = new Set([
  "apps/desktop/vite.config.d.ts",
  "apps/desktop/vite.config.js",
]);

function sourceError() {
  return new Error("macOS build source identity is invalid");
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

export function validateSourceIdentity(value) {
  if (
    !exactKeys(value, [
      "authority",
      "base_commit",
      "source_tree_sha256",
    ]) ||
    !SOURCE_AUTHORITIES.includes(value.authority) ||
    !SOURCE_COMMIT.test(value.base_commit) ||
    !DIGEST.test(value.source_tree_sha256)
  ) {
    throw sourceError();
  }
  return Object.freeze({ ...value });
}

function git(repoRoot, args, runner) {
  const result = runner(MACOS_SYSTEM_TOOLS.git, ["-C", repoRoot, ...args], {
    encoding: null,
    maxBuffer: MAX_GIT_OUTPUT_BYTES,
    timeout: 30_000,
  });
  if (
    result?.error ||
    result?.status !== 0 ||
    !Buffer.isBuffer(result.stdout) ||
    !Buffer.isBuffer(result.stderr) ||
    result.stderr.length !== 0 ||
    result.stdout.length > MAX_GIT_OUTPUT_BYTES
  ) {
    throw sourceError();
  }
  return result.stdout;
}

function defaultGitRunner(command, args, options) {
  return runClosedSystemTool(command, args, options);
}

function buildInput(relative) {
  if (ROOT_FILES.has(relative)) return true;
  if (!ROOT_PREFIXES.some((prefix) => relative.startsWith(prefix))) return false;
  return (
    !relative.split("/").some((part) => EXCLUDED_PARTS.has(part)) &&
    !relative.endsWith(".tsbuildinfo") &&
    !relative.startsWith("apps/desktop/src-tauri/gen/") &&
    !GENERATED_DESKTOP_FILES.has(relative)
  );
}

function decodeGitPaths(output) {
  if (output.length === 0) return [];
  if (output.at(-1) !== 0) throw sourceError();
  const entries = output.subarray(0, -1);
  const files = entries.length === 0 ? [] : entries.toString("utf8").split("\0");
  if (
    files.length > MAX_FILES ||
    files.some(
      (relative) =>
        !relative ||
        relative.includes("\\") ||
        path.posix.isAbsolute(relative) ||
        relative.split("/").some((part) => !part || part === "." || part === ".."),
    )
  ) {
    throw sourceError();
  }
  return files;
}

function decodeFileList(output) {
  const files = decodeGitPaths(output);
  if (files.length === 0) throw sourceError();
  return files
    .filter(buildInput)
    .sort((left, right) =>
      Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")),
    );
}

function updateLength(hash, value, bytes) {
  const buffer = Buffer.alloc(bytes);
  if (bytes === 4) buffer.writeUInt32BE(value);
  else buffer.writeBigUInt64BE(BigInt(value));
  hash.update(buffer);
}

async function digestFile(root, relative) {
  const file = path.join(root, ...relative.split("/"));
  let before;
  let resolved;
  let handle;
  try {
    [before, resolved] = await Promise.all([lstat(file), realpath(file)]);
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.size > MAX_FILE_BYTES ||
      resolved !== file
    ) {
      throw sourceError();
    }
    handle = await open(file, "r");
    const hash = createHash("sha256");
    const buffer = Buffer.alloc(1024 * 1024);
    let bytes = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length);
      if (bytesRead === 0) break;
      bytes += bytesRead;
      if (bytes > MAX_FILE_BYTES) throw sourceError();
      hash.update(buffer.subarray(0, bytesRead));
    }
    const after = await handle.stat();
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      before.mtimeMs !== after.mtimeMs ||
      bytes !== before.size
    ) {
      throw sourceError();
    }
    return Object.freeze({
      relative,
      bytes,
      executable: (before.mode & 0o111) !== 0,
      sha256: hash.digest(),
    });
  } catch (error) {
    if (error?.message === "macOS build source identity is invalid") throw error;
    throw sourceError();
  } finally {
    await handle?.close().catch(() => {});
  }
}

function sourceTreeSha256(records) {
  const tree = createHash("sha256");
  tree.update("resume-ir.macos-source-tree.v1\0");
  for (const record of records) {
    const relative = Buffer.from(record.relative, "utf8");
    updateLength(tree, relative.length, 4);
    tree.update(relative);
    tree.update(record.executable ? Buffer.from([1]) : Buffer.from([0]));
    updateLength(tree, record.bytes, 8);
    tree.update(record.sha256);
  }
  return tree.digest("hex");
}

async function enumerateSourceDirectory(root, relative) {
  let entries;
  try {
    entries = await readdir(path.join(root, relative), { withFileTypes: true });
  } catch {
    throw sourceError();
  }
  const files = [];
  entries.sort((left, right) =>
    Buffer.compare(
      Buffer.from(left.name, "utf8"),
      Buffer.from(right.name, "utf8"),
    ),
  );
  for (const entry of entries) {
    const child = path.posix.join(relative, entry.name);
    if (!buildInput(child)) {
      if (
        entry.isDirectory() &&
        ROOT_PREFIXES.some(
          (prefix) =>
            prefix.startsWith(`${child}/`) || child.startsWith(prefix),
        ) &&
        !EXCLUDED_PARTS.has(entry.name) &&
        child !== "apps/desktop/src-tauri/gen"
      ) {
        files.push(...(await enumerateSourceDirectory(root, child)));
      }
      continue;
    }
    if (entry.isSymbolicLink() || (!entry.isDirectory() && !entry.isFile())) {
      throw sourceError();
    }
    if (entry.isDirectory()) {
      files.push(...(await enumerateSourceDirectory(root, child)));
    } else {
      files.push(child);
    }
  }
  return files;
}

async function collectFilesystemBuildSource(root) {
  const files = [];
  for (const rootFile of [...ROOT_FILES].sort()) {
    try {
      const metadata = await lstat(path.join(root, rootFile));
      if (!metadata.isFile() || metadata.isSymbolicLink()) throw sourceError();
    } catch (error) {
      if (error?.message === "macOS build source identity is invalid") throw error;
      throw sourceError();
    }
    files.push(rootFile);
  }
  for (const rootDirectory of ["apps/desktop", "crates"]) {
    files.push(...(await enumerateSourceDirectory(root, rootDirectory)));
  }
  files.sort((left, right) =>
    Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8")),
  );
  if (files.length === 0 || files.length > MAX_FILES) throw sourceError();
  const records = [];
  let totalBytes = 0;
  for (const relative of files) {
    const record = await digestFile(root, relative);
    totalBytes += record.bytes;
    if (totalBytes > MAX_TOTAL_BYTES) throw sourceError();
    records.push(record);
  }
  return Object.freeze({
    records: Object.freeze(records),
    sourceTreeSha256: sourceTreeSha256(records),
  });
}

export async function collectBuildSource({
  repoRoot,
  runner = defaultGitRunner,
}) {
  if (!path.isAbsolute(repoRoot) || typeof runner !== "function") {
    throw sourceError();
  }
  let metadata;
  let resolvedRoot;
  try {
    [metadata, resolvedRoot] = await Promise.all([lstat(repoRoot), realpath(repoRoot)]);
  } catch {
    throw sourceError();
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    resolvedRoot !== repoRoot
  ) {
    throw sourceError();
  }
  const baseCommit = git(resolvedRoot, ["rev-parse", "--verify", "HEAD"], runner)
    .toString("ascii")
    .trim();
  if (!SOURCE_COMMIT.test(baseCommit)) throw sourceError();
  if (
    git(resolvedRoot, ["diff", "--name-only", "--diff-filter=U", "-z"], runner)
      .length !== 0
  ) {
    throw sourceError();
  }
  const deletedFiles = new Set(
    decodeGitPaths(
      git(resolvedRoot, ["ls-files", "--deleted", "-z"], runner),
    ),
  );
  const files = decodeFileList(
    git(
      resolvedRoot,
      ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
      runner,
    ),
  ).filter((relative) => !deletedFiles.has(relative));
  const collisionKeys = new Set();
  const records = [];
  let totalBytes = 0;
  for (const relative of files) {
    const collisionKey = relative.normalize("NFC").toLowerCase();
    if (collisionKeys.has(collisionKey)) throw sourceError();
    collisionKeys.add(collisionKey);
    const record = await digestFile(resolvedRoot, relative);
    totalBytes += record.bytes;
    if (totalBytes > MAX_TOTAL_BYTES) throw sourceError();
    records.push(record);
  }
  return Object.freeze({
    baseCommit,
    records: Object.freeze(records),
    sourceTreeSha256: sourceTreeSha256(records),
  });
}

export async function captureSourceIdentity({
  repoRoot,
  authority,
  runner = defaultGitRunner,
  verifyMain = verifyMainSourceProvenance,
}) {
  if (!SOURCE_AUTHORITIES.includes(authority)) throw sourceError();
  if (authority === "exact_main_commit") {
    const verified = await verifyMain({ repoRoot });
    if (!SOURCE_COMMIT.test(verified ?? "")) throw sourceError();
  }
  const source = await collectBuildSource({ repoRoot, runner });
  const identity = validateSourceIdentity({
    authority,
    base_commit: source.baseCommit,
    source_tree_sha256: source.sourceTreeSha256,
  });
  return Object.freeze({ identity, records: source.records });
}

export async function verifySourceIdentity({
  repoRoot,
  expected,
  runner = defaultGitRunner,
  verifyMain = verifyMainSourceProvenance,
}) {
  const identity = validateSourceIdentity(expected);
  const observed = await captureSourceIdentity({
    repoRoot,
    authority: identity.authority,
    runner,
    verifyMain,
  });
  if (JSON.stringify(observed.identity) !== JSON.stringify(identity)) {
    throw sourceError();
  }
  return identity;
}

export async function verifyImmutableSnapshotSource({
  repoRoot,
  expected,
}) {
  const identity = validateSourceIdentity(expected);
  if (identity.authority !== "worktree_snapshot") throw sourceError();
  if (!path.isAbsolute(repoRoot)) throw sourceError();
  let metadata;
  let resolvedRoot;
  try {
    [metadata, resolvedRoot] = await Promise.all([lstat(repoRoot), realpath(repoRoot)]);
  } catch {
    throw sourceError();
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    resolvedRoot !== repoRoot
  ) {
    throw sourceError();
  }
  const observed = await collectFilesystemBuildSource(resolvedRoot);
  if (observed.sourceTreeSha256 !== identity.source_tree_sha256) {
    throw sourceError();
  }
  return identity;
}
