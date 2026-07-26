import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { verifyImmutableSnapshotSource } from "./macos-source-identity.mjs";
import { createImmutableWorktreeSnapshot } from "./macos-worktree-build-source.mjs";

function git(args, cwd) {
  const result = spawnSync("/usr/bin/git", args, {
    cwd,
    encoding: "utf8",
    shell: false,
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

async function repositoryFixture(context) {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-worktree-snapshot-"),
  );
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const root = await realpath(temporary);
  const repoRoot = path.join(root, "repo");
  const cacheRoot = path.join(root, "cache");
  await mkdir(path.join(repoRoot, "apps", "desktop"), { recursive: true });
  await mkdir(path.join(repoRoot, "crates", "sample", "src"), {
    recursive: true,
  });
  await writeFile(path.join(repoRoot, "Cargo.lock"), "# synthetic\n");
  await writeFile(path.join(repoRoot, "Cargo.toml"), "[workspace]\n");
  await writeFile(
    path.join(repoRoot, "apps", "desktop", "package.json"),
    '{"private":true}\n',
  );
  await writeFile(
    path.join(repoRoot, "crates", "sample", "src", "lib.rs"),
    "pub fn value() -> u8 { 1 }\n",
  );
  git(["init"], repoRoot);
  git(["config", "user.email", "synthetic@example.invalid"], repoRoot);
  git(["config", "user.name", "Synthetic Test"], repoRoot);
  git(["add", "."], repoRoot);
  git(["commit", "-m", "fixture"], repoRoot);
  return { cacheRoot, repoRoot };
}

test("copies the dirty worktree into one content-addressed immutable source", async (context) => {
  const { cacheRoot, repoRoot } = await repositoryFixture(context);
  const sourceFile = path.join(repoRoot, "crates", "sample", "src", "lib.rs");
  await writeFile(sourceFile, "pub fn value() -> u8 { 2 }\n");
  await writeFile(
    path.join(repoRoot, "apps", "desktop", "untracked.mjs"),
    "export const included = true;\n",
  );

  const first = await createImmutableWorktreeSnapshot({ repoRoot, cacheRoot });
  assert.equal(first.reused, false);
  assert.equal(first.source.authority, "worktree_snapshot");
  await assert.doesNotReject(
    verifyImmutableSnapshotSource({
      repoRoot: first.repoRoot,
      expected: first.source,
    }),
  );

  await writeFile(sourceFile, "source drift after snapshot\n");
  assert.match(
    await readFile(
      path.join(first.repoRoot, "crates", "sample", "src", "lib.rs"),
      "utf8",
    ),
    /value\(\) -> u8 \{ 2 \}/,
  );
  await writeFile(sourceFile, "pub fn value() -> u8 { 2 }\n");
  const second = await createImmutableWorktreeSnapshot({ repoRoot, cacheRoot });
  assert.equal(second.reused, true);
  assert.equal(second.repoRoot, first.repoRoot);
  assert.deepEqual(second.source, first.source);
});

test("rejects a mutated cached source instead of silently rebuilding over it", async (context) => {
  const { cacheRoot, repoRoot } = await repositoryFixture(context);
  const snapshot = await createImmutableWorktreeSnapshot({ repoRoot, cacheRoot });
  await writeFile(
    path.join(snapshot.repoRoot, "crates", "sample", "src", "lib.rs"),
    "tampered\n",
  );
  await assert.rejects(
    createImmutableWorktreeSnapshot({ repoRoot, cacheRoot }),
    /worktree build snapshot is invalid/,
  );
});
