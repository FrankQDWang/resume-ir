import { spawnSync } from "node:child_process";
import { lstat, realpath } from "node:fs/promises";
import path from "node:path";

import { MACOS_SYSTEM_TOOLS } from "./macos-system-tools.mjs";

const SOURCE_COMMIT = /^[a-f0-9]{40}$/;
const MAX_GIT_OUTPUT_BYTES = 1024 * 1024;
const EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git";
const GIT_ENVIRONMENT = Object.freeze({
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_NO_REPLACE_OBJECTS: "1",
  GIT_OPTIONAL_LOCKS: "0",
  GIT_TERMINAL_PROMPT: "0",
  HOME: "/var/empty",
  LANG: "C",
  LC_ALL: "C",
  PATH: "/usr/bin:/bin",
});

function provenanceError() {
  return new Error("macOS build source provenance is invalid");
}

function defaultGitRunner(command, args) {
  return spawnSync(command, args, {
    encoding: "utf8",
    env: GIT_ENVIRONMENT,
    maxBuffer: MAX_GIT_OUTPUT_BYTES,
    timeout: 30_000,
    shell: false,
    windowsHide: true,
  });
}

function succeeded(result) {
  return !result?.error && result?.status === 0;
}

async function runGit(repoRoot, args, runner) {
  let result;
  try {
    result = await runner(MACOS_SYSTEM_TOOLS.git, [
      "-C",
      repoRoot,
      ...args,
    ]);
  } catch {
    throw provenanceError();
  }
  const output = `${result?.stdout ?? ""}${result?.stderr ?? ""}`;
  if (Buffer.byteLength(output, "utf8") > MAX_GIT_OUTPUT_BYTES) {
    throw provenanceError();
  }
  return result;
}

export async function verifyMainSourceProvenance({
  repoRoot,
  runner = defaultGitRunner,
}) {
  if (!path.isAbsolute(repoRoot) || typeof runner !== "function") {
    throw provenanceError();
  }
  let metadata;
  let resolvedRoot;
  try {
    [metadata, resolvedRoot] = await Promise.all([
      lstat(repoRoot),
      realpath(repoRoot),
    ]);
  } catch {
    throw provenanceError();
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    resolvedRoot !== path.resolve(repoRoot)
  ) {
    throw provenanceError();
  }

  const topLevel = await runGit(
    resolvedRoot,
    ["rev-parse", "--show-toplevel"],
    runner,
  );
  if (
    !succeeded(topLevel) ||
    topLevel.stdout !== `${resolvedRoot}\n`
  ) {
    throw provenanceError();
  }

  const origin = await runGit(
    resolvedRoot,
    ["remote", "get-url", "--all", "origin"],
    runner,
  );
  if (!succeeded(origin) || origin.stdout !== `${EXPECTED_ORIGIN}\n`) {
    throw provenanceError();
  }
  const configuredOrigin = await runGit(
    resolvedRoot,
    ["config", "--local", "--get-all", "remote.origin.url"],
    runner,
  );
  if (
    !succeeded(configuredOrigin) ||
    configuredOrigin.stdout !== `${EXPECTED_ORIGIN}\n`
  ) {
    throw provenanceError();
  }

  const head = await runGit(resolvedRoot, ["rev-parse", "HEAD"], runner);
  const sourceCommit = `${head.stdout ?? ""}`.trim();
  if (!succeeded(head) || !SOURCE_COMMIT.test(sourceCommit)) {
    throw provenanceError();
  }

  const branch = await runGit(
    resolvedRoot,
    ["symbolic-ref", "--quiet", "--short", "HEAD"],
    runner,
  );
  if (succeeded(branch)) {
    if (branch.stdout !== "main\n") throw provenanceError();
  } else if (branch?.error || branch?.status !== 1) {
    throw provenanceError();
  }

  const status = await runGit(
    resolvedRoot,
    ["status", "--porcelain=v1", "--untracked-files=all"],
    runner,
  );
  if (!succeeded(status) || status.stdout !== "") throw provenanceError();

  const remote = await runGit(
    resolvedRoot,
    ["ls-remote", "origin", "refs/heads/main"],
    runner,
  );
  if (
    !succeeded(remote) ||
    remote.stdout !== `${sourceCommit}\trefs/heads/main\n`
  ) {
    throw provenanceError();
  }
  return sourceCommit;
}
