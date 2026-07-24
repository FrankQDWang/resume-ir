import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  realpath,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  captureSourceIdentity,
  validateSourceIdentity,
  verifyImmutableSnapshotSource,
} from "./macos-source-identity.mjs";

function git(args, cwd) {
  const result = spawnSync("/usr/bin/git", args, {
    cwd,
    encoding: "utf8",
    shell: false,
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  return result.stdout.trim();
}

async function repositoryFixture(context) {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-source-identity-"),
  );
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const root = await realpath(temporary);
  git(["init"], root);
  git(["config", "user.email", "synthetic@example.invalid"], root);
  git(["config", "user.name", "Synthetic Test"], root);
  await mkdir(path.join(root, "apps", "desktop"), { recursive: true });
  await mkdir(path.join(root, "crates", "sample", "src"), { recursive: true });
  await writeFile(path.join(root, "Cargo.lock"), "# synthetic\n");
  await writeFile(path.join(root, "Cargo.toml"), "[workspace]\n");
  await writeFile(
    path.join(root, "apps", "desktop", "package.json"),
    '{"private":true}\n',
  );
  await writeFile(
    path.join(root, "crates", "sample", "src", "lib.rs"),
    "pub fn value() -> u8 { 1 }\n",
  );
  git(["add", "."], root);
  git(["commit", "-m", "fixture"], root);
  return root;
}

test("worktree identity includes tracked changes and untracked build inputs", async (context) => {
  const repoRoot = await repositoryFixture(context);
  const original = await captureSourceIdentity({
    repoRoot,
    authority: "worktree_snapshot",
  });
  assert.equal(original.identity.base_commit, git(["rev-parse", "HEAD"], repoRoot));
  await assert.doesNotReject(
    verifyImmutableSnapshotSource({
      repoRoot,
      expected: original.identity,
    }),
  );

  await writeFile(
    path.join(repoRoot, "crates", "sample", "src", "lib.rs"),
    "pub fn value() -> u8 { 2 }\n",
  );
  await writeFile(
    path.join(repoRoot, "apps", "desktop", "untracked.mjs"),
    "export const included = true;\n",
  );
  const changed = await captureSourceIdentity({
    repoRoot,
    authority: "worktree_snapshot",
  });
  assert.notEqual(
    changed.identity.source_tree_sha256,
    original.identity.source_tree_sha256,
  );
  assert.deepEqual(
    changed.records.map(({ relative }) => relative),
    [
      "Cargo.lock",
      "Cargo.toml",
      "apps/desktop/package.json",
      "apps/desktop/untracked.mjs",
      "crates/sample/src/lib.rs",
    ],
  );
});

test("source identity is a hard-cut closed contract", () => {
  assert.throws(
    () =>
      validateSourceIdentity({
        authority: "worktree_snapshot",
        base_commit: "1".repeat(40),
        source_tree_sha256: "2".repeat(64),
        source_commit: "1".repeat(40),
      }),
    /source identity is invalid/,
  );
  assert.throws(
    () =>
      validateSourceIdentity({
        authority: "legacy",
        base_commit: "1".repeat(40),
        source_tree_sha256: "2".repeat(64),
      }),
    /source identity is invalid/,
  );
});

test("worktree identity rejects symlinked build inputs", async (context) => {
  const repoRoot = await repositoryFixture(context);
  await symlink(
    path.join(repoRoot, "Cargo.toml"),
    path.join(repoRoot, "apps", "desktop", "linked-input"),
  );
  await assert.rejects(
    captureSourceIdentity({
      repoRoot,
      authority: "worktree_snapshot",
    }),
    /source identity is invalid/,
  );
});

test("worktree identity represents tracked deletions instead of reading removed paths", async (context) => {
  const repoRoot = await repositoryFixture(context);
  await unlink(path.join(repoRoot, "apps", "desktop", "package.json"));

  const captured = await captureSourceIdentity({
    repoRoot,
    authority: "worktree_snapshot",
  });

  assert.deepEqual(
    captured.records.map(({ relative }) => relative),
    [
      "Cargo.lock",
      "Cargo.toml",
      "crates/sample/src/lib.rs",
    ],
  );
  await assert.doesNotReject(
    verifyImmutableSnapshotSource({
      repoRoot,
      expected: captured.identity,
    }),
  );
});
