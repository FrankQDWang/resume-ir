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

import {
  buildMacosWorktreeRelease,
  resolveMacosWorktreeRepoRoot,
} from "./macos-worktree-release.mjs";
import { sha256 } from "./verify-bundled-sidecar.mjs";

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
    path.join(os.tmpdir(), "resume-ir-worktree-release-"),
  );
  context.after(() => rm(temporary, { recursive: true, force: true }));
  const root = await realpath(temporary);
  const repoRoot = path.join(root, "repo");
  const frontendRoot = path.join(repoRoot, "apps", "desktop");
  const tauriRoot = path.join(frontendRoot, "src-tauri");
  await mkdir(tauriRoot, { recursive: true });
  await mkdir(path.join(repoRoot, "crates", "sample", "src"), {
    recursive: true,
  });
  await writeFile(path.join(repoRoot, "Cargo.lock"), "# synthetic\n");
  await writeFile(path.join(repoRoot, "Cargo.toml"), "[workspace]\n");
  await writeFile(
    path.join(frontendRoot, "package.json"),
    '{"private":true}\n',
  );
  await writeFile(
    path.join(frontendRoot, "package-lock.json"),
    '{"lockfileVersion":3}\n',
  );
  await writeFile(
    path.join(tauriRoot, "tauri.conf.json"),
    '{"productName":"resume-ir","version":"0.1.2"}\n',
  );
  await writeFile(
    path.join(tauriRoot, "tauri.macos.conf.json"),
    '{"bundle":{"targets":["dmg"],"macOS":{"signingIdentity":"-","hardenedRuntime":true}}}\n',
  );
  const sourceFile = path.join(repoRoot, "crates", "sample", "src", "lib.rs");
  await writeFile(sourceFile, "pub fn value() -> u8 { 1 }\n");
  git(["init"], repoRoot);
  git(["config", "user.email", "synthetic@example.invalid"], repoRoot);
  git(["config", "user.name", "Synthetic Test"], repoRoot);
  git(["add", "."], repoRoot);
  git(["commit", "-m", "fixture"], repoRoot);
  await writeFile(sourceFile, "pub fn value() -> u8 { 2 }\n");
  return { repoRoot, root };
}

test("resolves the CLI worktree root to one canonical absolute path", () => {
  const repoRoot = resolveMacosWorktreeRepoRoot();
  assert.equal(repoRoot, path.resolve(repoRoot));
});

test("builds and publishes a DMG only from the immutable worktree snapshot", async (context) => {
  const { repoRoot, root } = await repositoryFixture(context);
  const artifactRoot = path.join(root, "artifacts");
  const cacheRoot = path.join(root, "cache");
  let releaseRepoRoot;
  const result = await buildMacosWorktreeRelease({
    repoRoot,
    artifactRoot,
    cacheRoot,
    platform: "darwin",
    installDependencies: async () => ({ status: 0 }),
    stageRuntimePacks: async ({ immutableRepoRoot, sourceRepoRoot }) => {
      releaseRepoRoot = immutableRepoRoot;
      assert.notEqual(immutableRepoRoot, sourceRepoRoot);
    },
    runRelease: async ({ plan, source }) => {
      await mkdir(path.dirname(plan.dmg), { recursive: true });
      await writeFile(plan.dmg, "synthetic-worktree-dmg");
      return {
        status: 0,
        stdout: `${JSON.stringify({
          schema_version: "resume-ir.macos-dmg-composition.v3",
          target_triple: "aarch64-apple-darwin",
          source,
          dmg_sha256: await sha256(plan.dmg),
          release_claim: "composition_only",
        })}\n`,
        stderr: "",
      };
    },
  });

  assert.equal(result.receipt.source.authority, "worktree_snapshot");
  assert.equal(result.receipt.artifact_file, path.basename(result.artifact));
  assert.equal(
    await readFile(
      path.join(releaseRepoRoot, "crates", "sample", "src", "lib.rs"),
      "utf8",
    ),
    "pub fn value() -> u8 { 2 }\n",
  );
  assert.equal(await sha256(result.artifact), result.receipt.dmg_sha256);
  assert.deepEqual(
    JSON.parse(await readFile(`${result.artifact}.json`, "utf8")),
    result.receipt,
  );
});
