import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { runBoundedTool, toolSucceeded } from "./bounded-process.mjs";
import { createImmutableBuildSource } from "./immutable-build-source.mjs";
import { stageImmutableRuntimePacks } from "./immutable-runtime-packs.mjs";

async function git(repo, args) {
  const result = await runBoundedTool("/usr/bin/git", ["-C", repo, ...args], {
    env: { HOME: "/var/empty", LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin" },
    timeoutMs: 10_000,
  });
  assert.equal(toolSucceeded(result), true, result.stderr);
  return result.stdout.trim();
}

test("stages only manifest-reviewed runtime inputs into the exact build clone", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-immutable-packs-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const immutableRepoRoot = path.join(root, "immutable");
  const sourceRepoRoot = path.join(root, "source");
  await Promise.all([mkdir(immutableRepoRoot), mkdir(sourceRepoRoot)]);
  const calls = [];
  const resource = (kind) => ({
    destination: `/ignored/${kind}`,
    expectedManifest: `/exact/${kind}.json`,
    sourcePackRoot: `/reviewed/${kind}`,
    targetTriple: "aarch64-apple-darwin",
  });
  await stageImmutableRuntimePacks(
    { immutableRepoRoot, sourceRepoRoot },
    {
      createPlan: (options) => {
        assert.equal(options.repoRoot, immutableRepoRoot);
        assert.equal(options.targetTriple, "aarch64-apple-darwin");
        assert.equal(
          options.sourcePackRoot,
          path.join(sourceRepoRoot, ".cache", "resume-ir-native-e5-qint8-pack"),
        );
        return {
          classifierResourcePack: resource("classifier"),
          ocrResourcePack: resource("ocr"),
          resourcePack: resource("embedding"),
        };
      },
      stageClassifier: async (plan) => calls.push(["classifier", plan]),
      stageEmbedding: async (plan) => calls.push(["embedding", plan]),
      stageOcr: async (plan) => calls.push(["ocr", plan]),
    },
  );
  assert.deepEqual(
    calls.map(([kind, plan]) => [
      kind,
      path.relative(immutableRepoRoot, plan.destination),
    ]),
    [
      ["embedding", ".cache/resume-ir-native-e5-qint8-pack"],
      ["ocr", ".cache/resume-ir-macos-ocr-runtime-pack"],
      ["classifier", ".cache/resume-ir-classifier-model-pack"],
    ],
  );
  assert.deepEqual(
    calls.map(([, plan]) => plan.expectedManifest),
    ["/exact/embedding.json", "/exact/ocr.json", "/exact/classifier.json"],
  );
});

test("creates and removes an inode-bound exact-commit build clone", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-build-source-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const repo = path.join(root, "repo");
  const temporaryParent = path.join(root, "temporary");
  await mkdir(path.join(repo, "apps", "desktop"), { recursive: true });
  await mkdir(temporaryParent, { mode: 0o700 });
  await chmod(temporaryParent, 0o700);
  const packageFile = path.join(repo, "apps", "desktop", "package.json");
  await writeFile(
    packageFile,
    `${JSON.stringify({ name: "immutable-fixture", version: "1.0.0" })}\n`,
  );
  await writeFile(
    path.join(repo, "apps", "desktop", "package-lock.json"),
    `${JSON.stringify({
      name: "immutable-fixture",
      version: "1.0.0",
      lockfileVersion: 3,
      requires: true,
      packages: { "": { name: "immutable-fixture", version: "1.0.0" } },
    })}\n`,
  );
  await git(repo, ["init", "--quiet"]);
  await git(repo, ["config", "user.name", "Synthetic Test"]);
  await git(repo, ["config", "user.email", "synthetic@example.test"]);
  await git(repo, ["add", "--", "."]);
  await git(repo, ["commit", "--quiet", "-m", "synthetic fixture"]);
  const head = await git(repo, ["rev-parse", "HEAD"]);

  let staged = false;
  const immutable = await createImmutableBuildSource({
    repoRoot: repo,
    runTool: runBoundedTool,
    source: { gitHead: head },
    stageRuntimePacks: async ({ immutableRepoRoot, sourceRepoRoot }) => {
      assert.notEqual(immutableRepoRoot, repo);
      assert.equal(sourceRepoRoot, repo);
      staged = true;
    },
    temporaryParent,
  });
  assert.equal(staged, true);
  assert.notEqual(immutable.repoRoot, repo);
  assert.equal(await git(immutable.repoRoot, ["rev-parse", "HEAD"]), head);
  await writeFile(packageFile, "source drift that must not reach the clone\n");
  assert.match(
    await readFile(
      path.join(immutable.repoRoot, "apps", "desktop", "package.json"),
      "utf8",
    ),
    /immutable-fixture/,
  );
  await immutable.cleanup();
  await assert.rejects(readFile(immutable.repoRoot), { code: "ENOENT" });
});

test("pins npm and the Tauri build PATH outside ignored source dependencies", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-build-toolchain-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const repo = path.join(root, "repo");
  const temporaryParent = path.join(root, "temporary");
  const frontendRoot = path.join(repo, "apps", "desktop");
  const fakeBin = path.join(frontendRoot, "node_modules", ".bin");
  const sentinel = path.join(root, "fake-npm-ran");
  await mkdir(frontendRoot, { recursive: true });
  await mkdir(temporaryParent, { mode: 0o700 });
  await chmod(temporaryParent, 0o700);
  await writeFile(path.join(repo, ".gitignore"), "node_modules/\n");
  await writeFile(
    path.join(frontendRoot, "package.json"),
    `${JSON.stringify({ name: "immutable-fixture", version: "1.0.0" })}\n`,
  );
  await writeFile(
    path.join(frontendRoot, "package-lock.json"),
    `${JSON.stringify({
      name: "immutable-fixture",
      version: "1.0.0",
      lockfileVersion: 3,
      requires: true,
      packages: { "": { name: "immutable-fixture", version: "1.0.0" } },
    })}\n`,
  );
  await git(repo, ["init", "--quiet"]);
  await git(repo, ["config", "user.name", "Synthetic Test"]);
  await git(repo, ["config", "user.email", "synthetic@example.test"]);
  await git(repo, ["add", "--", "."]);
  await git(repo, ["commit", "--quiet", "-m", "synthetic fixture"]);
  const head = await git(repo, ["rev-parse", "HEAD"]);
  await mkdir(fakeBin, { recursive: true });
  await writeFile(
    path.join(fakeBin, "npm"),
    "#!/bin/sh\nprintf invoked > \"$FAKE_NPM_SENTINEL\"\n",
    { mode: 0o700 },
  );

  const moduleUrl = new URL("./immutable-build-source.mjs", import.meta.url).href;
  const boundedUrl = new URL("./bounded-process.mjs", import.meta.url).href;
  const child = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `
        const [{ createImmutableBuildSource }, { runBoundedTool, toolSucceeded }] =
          await Promise.all([import(process.argv[1]), import(process.argv[2])]);
        const immutable = await createImmutableBuildSource({
          repoRoot: process.argv[3],
          runTool: runBoundedTool,
          source: { gitHead: process.argv[4] },
          stageRuntimePacks: async () => {},
          temporaryParent: process.argv[5],
        });
        const npm = await runBoundedTool("/usr/bin/which", ["npm"], {
          env: immutable.buildEnvironment,
          timeoutMs: 10_000,
        });
        if (!toolSucceeded(npm)) throw new Error("npm lookup failed");
        const npmVersion = await runBoundedTool(npm.stdout.trim(), ["--version"], {
          env: immutable.buildEnvironment,
          timeoutMs: 10_000,
        });
        if (!toolSucceeded(npmVersion)) throw new Error("npm execution failed");
        const output = {
          buildEnvironment: immutable.buildEnvironment,
          npmPath: npm.stdout.trim(),
          npmVersion: npmVersion.stdout.trim(),
        };
        await immutable.cleanup();
        process.stdout.write(JSON.stringify(output));
      `,
      moduleUrl,
      boundedUrl,
      repo,
      head,
      temporaryParent,
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        FAKE_NPM_SENTINEL: sentinel,
        HOME: path.join(repo, "ignored-home"),
        NODE_OPTIONS: "--no-warnings",
        PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
      },
    },
  );
  assert.equal(child.status, 0, child.stderr);
  await assert.rejects(readFile(sentinel), { code: "ENOENT" });
  const observed = JSON.parse(child.stdout);
  assert.equal(observed.npmPath.startsWith(temporaryParent), true);
  assert.equal(observed.npmPath.includes("node_modules/.bin"), false);
  assert.match(observed.npmVersion, /^(0|[1-9][0-9]*)\.[0-9]+\.[0-9]+$/);
  assert.equal(observed.buildEnvironment.PATH.includes(fakeBin), false);
  assert.equal("NODE_OPTIONS" in observed.buildEnvironment, false);
  assert.deepEqual(Object.keys(observed.buildEnvironment).sort(), [
    "CARGO_HOME",
    "HOME",
    "LANG",
    "LC_ALL",
    "NPM_CONFIG_CACHE",
    "NPM_CONFIG_GLOBALCONFIG",
    "NPM_CONFIG_SCRIPT_SHELL",
    "NPM_CONFIG_UPDATE_NOTIFIER",
    "NPM_CONFIG_USERCONFIG",
    "PATH",
    "RUSTUP_HOME",
    "TMPDIR",
  ]);
});

test("canonicalizes Rust shims before exposing the closed build PATH", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-build-rust-shim-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const repo = path.join(root, "repo");
  const temporaryParent = path.join(root, "temporary");
  const runtimeHome = path.join(root, "runtime-home");
  const cargoBin = path.join(runtimeHome, ".cargo", "bin");
  await mkdir(path.join(repo, "apps", "desktop"), { recursive: true });
  await mkdir(temporaryParent, { mode: 0o700 });
  await mkdir(cargoBin, { recursive: true, mode: 0o700 });
  await mkdir(path.join(runtimeHome, ".rustup"), { mode: 0o700 });
  await chmod(temporaryParent, 0o700);
  await chmod(runtimeHome, 0o700);
  await chmod(path.join(runtimeHome, ".cargo"), 0o700);
  await chmod(cargoBin, 0o700);
  await symlink(process.execPath, path.join(cargoBin, "cargo"));
  await symlink(process.execPath, path.join(cargoBin, "rustc"));
  await writeFile(
    path.join(repo, "apps", "desktop", "package.json"),
    `${JSON.stringify({ name: "immutable-fixture", version: "1.0.0" })}\n`,
  );
  await writeFile(
    path.join(repo, "apps", "desktop", "package-lock.json"),
    `${JSON.stringify({
      name: "immutable-fixture",
      version: "1.0.0",
      lockfileVersion: 3,
      requires: true,
      packages: { "": { name: "immutable-fixture", version: "1.0.0" } },
    })}\n`,
  );
  await git(repo, ["init", "--quiet"]);
  await git(repo, ["config", "user.name", "Synthetic Test"]);
  await git(repo, ["config", "user.email", "synthetic@example.test"]);
  await git(repo, ["add", "--", "."]);
  await git(repo, ["commit", "--quiet", "-m", "synthetic fixture"]);
  const head = await git(repo, ["rev-parse", "HEAD"]);

  const immutable = await createImmutableBuildSource({
    repoRoot: repo,
    runTool: runBoundedTool,
    runtime: { homeDirectory: runtimeHome, nodeExecutable: process.execPath },
    source: { gitHead: head },
    stageRuntimePacks: async () => {},
    temporaryParent,
  });
  const toolBin = immutable.buildEnvironment.PATH.split(":")[0];
  const canonicalNode = await realpath(process.execPath);
  const replacement = path.join(root, "replacement-cargo");
  await writeFile(replacement, "#!/bin/sh\nexit 91\n", { mode: 0o700 });
  await unlink(path.join(cargoBin, "cargo"));
  await symlink(replacement, path.join(cargoBin, "cargo"));
  assert.equal(await realpath(path.join(toolBin, "cargo")), canonicalNode);
  await immutable.cleanup();
});
