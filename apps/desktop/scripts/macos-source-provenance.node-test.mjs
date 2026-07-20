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

import { verifyMainSourceProvenance } from "./macos-source-provenance.mjs";

const EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git";

function git(args, cwd) {
  const result = spawnSync("/usr/bin/git", args, {
    cwd,
    encoding: "utf8",
    shell: false,
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  return result.stdout.trim();
}

function provenanceRunner({
  originOutput = `${EXPECTED_ORIGIN}\n`,
  configuredOriginOutput = `${EXPECTED_ORIGIN}\n`,
} = {}) {
  return (command, args) => {
    assert.equal(command, "/usr/bin/git");
    if (
      JSON.stringify(args.slice(2)) ===
      JSON.stringify(["remote", "get-url", "--all", "origin"])
    ) {
      return { status: 0, stdout: originOutput, stderr: "" };
    }
    if (
      JSON.stringify(args.slice(2)) ===
      JSON.stringify([
        "config",
        "--local",
        "--get-all",
        "remote.origin.url",
      ])
    ) {
      return { status: 0, stdout: configuredOriginOutput, stderr: "" };
    }
    return spawnSync(command, args, {
      encoding: "utf8",
      maxBuffer: 1024 * 1024,
      shell: false,
    });
  };
}

async function repositoryFixture(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-source-proof-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const origin = path.join(root, "origin.git");
  const checkout = path.join(root, "repo");
  await mkdir(origin);
  git(["init", "--bare"], origin);
  git(["clone", origin, checkout], root);
  const repo = await realpath(checkout);
  git(["checkout", "-b", "main"], repo);
  git(["config", "user.email", "synthetic@example.invalid"], repo);
  git(["config", "user.name", "Synthetic Test"], repo);
  await writeFile(path.join(repo, "tracked.txt"), "one\n");
  git(["add", "tracked.txt"], repo);
  git(["commit", "-m", "initial"], repo);
  git(["push", "-u", "origin", "main"], repo);
  return { root, repo };
}

test("accepts a clean main checkout and detached exact origin/main", async (context) => {
  const { repo } = await repositoryFixture(context);
  const expected = git(["rev-parse", "HEAD"], repo);
  assert.equal(
    await verifyMainSourceProvenance({
      repoRoot: repo,
      runner: provenanceRunner(),
    }),
    expected,
  );

  git(["checkout", "--detach", expected], repo);
  assert.equal(
    await verifyMainSourceProvenance({
      repoRoot: repo,
      runner: provenanceRunner(),
    }),
    expected,
  );
});

test("ignores a forged git executable injected through PATH", async (context) => {
  const { root, repo } = await repositoryFixture(context);
  const expected = git(["rev-parse", "HEAD"], repo);
  const fakeBin = path.join(root, "fake-bin");
  const marker = path.join(root, "forged-git-ran");
  await mkdir(fakeBin);
  await writeFile(
    path.join(fakeBin, "git"),
    `#!/bin/sh\nprintf forged > "${marker}"\nexit 42\n`,
    { mode: 0o755 },
  );
  const moduleUrl = new URL("./macos-source-provenance.mjs", import.meta.url).href;
  const child = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `import { spawnSync } from "node:child_process"; const [moduleUrl, repoRoot] = process.argv.slice(1); const { verifyMainSourceProvenance } = await import(moduleUrl); const expected = ${JSON.stringify(`${EXPECTED_ORIGIN}\n`)}; const runner = (command, args) => { const operation = JSON.stringify(args.slice(2)); if (operation === JSON.stringify(["remote", "get-url", "--all", "origin"]) || operation === JSON.stringify(["config", "--local", "--get-all", "remote.origin.url"])) return { status: 0, stdout: expected, stderr: "" }; return spawnSync(command, args, { encoding: "utf8", env: process.env, shell: false }); }; process.stdout.write(await verifyMainSourceProvenance({ repoRoot, runner }));`,
      moduleUrl,
      repo,
    ],
    {
      encoding: "utf8",
      env: { ...process.env, PATH: fakeBin },
      shell: false,
    },
  );
  assert.equal(child.status, 0, `${child.stdout}\n${child.stderr}`);
  assert.equal(child.stdout, expected);
  await assert.rejects(readFile(marker), { code: "ENOENT" });
});

test("rejects tracked, untracked, branch, and remote-main drift", async (context) => {
  const { repo } = await repositoryFixture(context);
  const runner = provenanceRunner();

  await writeFile(path.join(repo, "tracked.txt"), "dirty\n");
  await assert.rejects(
    verifyMainSourceProvenance({ repoRoot: repo, runner }),
    /source provenance is invalid/,
  );
  git(["restore", "tracked.txt"], repo);

  await writeFile(path.join(repo, "untracked-build-input.txt"), "untracked\n");
  await assert.rejects(
    verifyMainSourceProvenance({ repoRoot: repo, runner }),
    /source provenance is invalid/,
  );
  await rm(path.join(repo, "untracked-build-input.txt"));

  git(["checkout", "-b", "feature"], repo);
  await assert.rejects(
    verifyMainSourceProvenance({ repoRoot: repo, runner }),
    /source provenance is invalid/,
  );
  git(["checkout", "main"], repo);

  await writeFile(path.join(repo, "tracked.txt"), "two\n");
  git(["add", "tracked.txt"], repo);
  git(["commit", "-m", "remote ahead"], repo);
  git(["push", "origin", "main"], repo);
  git(["checkout", "--detach", "HEAD^"], repo);
  await assert.rejects(
    verifyMainSourceProvenance({ repoRoot: repo, runner }),
    /source provenance is invalid/,
  );
});

test("rejects wrong, rewritten, or multiple origin URLs", async (context) => {
  const { repo } = await repositoryFixture(context);
  for (const outputs of [
    { originOutput: "git@github.com:FrankQDWang/resume-ir.git\n" },
    { originOutput: "https://github.com/another/repository.git\n" },
    {
      originOutput: `${EXPECTED_ORIGIN}\nhttps://github.com/another/repository.git\n`,
    },
    {
      configuredOriginOutput:
        "git@github.com:FrankQDWang/resume-ir.git\n",
    },
    {
      configuredOriginOutput:
        `${EXPECTED_ORIGIN}\nhttps://github.com/another/repository.git\n`,
    },
  ]) {
    await assert.rejects(
      verifyMainSourceProvenance({
        repoRoot: repo,
        runner: provenanceRunner(outputs),
      }),
      /source provenance is invalid/,
    );
  }
});
