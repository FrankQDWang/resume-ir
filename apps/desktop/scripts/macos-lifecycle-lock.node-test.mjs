import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { spawn } from "node:child_process";
import test from "node:test";

import {
  acquireLifecycleLock,
  LIFECYCLE_LOCK_FILE,
  prepareLifecycleLockFile,
  releaseLifecycleLock,
  requireLifecycleLockCapability,
} from "./macos-lifecycle-lock.mjs";

const darwinTest = process.platform === "darwin" ? test : test.skip;

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-lifecycle-lock-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationSupportRoot = path.join(root, "Library", "Application Support");
  const evidenceDirectory = path.join(
    applicationSupportRoot,
    "local.resume-ir.desktop",
  );
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const prepareEvidenceDirectory = async (receivedRoot) => {
    assert.equal(receivedRoot, applicationSupportRoot);
    await mkdir(evidenceDirectory, { mode: 0o700 }).catch((error) => {
      if (error?.code !== "EEXIST") throw error;
    });
    return realpath(evidenceDirectory);
  };
  return {
    applicationSupportRoot,
    evidenceDirectory,
    prepareEvidenceDirectory,
    root,
  };
}

async function prepareLock(values) {
  return prepareLifecycleLockFile({
    applicationSupportRoot: values.applicationSupportRoot,
    prepareEvidenceDirectory: values.prepareEvidenceDirectory,
  });
}

function waitForLine(stream, expected, timeoutMilliseconds = 3_000) {
  return new Promise((resolve, reject) => {
    let source = "";
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error("helper readiness timed out"));
    }, timeoutMilliseconds);
    const consume = (chunk) => {
      source += chunk.toString("utf8");
      if (source.length > 128) {
        cleanup();
        reject(new Error("helper readiness was oversized"));
        return;
      }
      if (source.includes("\n")) {
        cleanup();
        if (source === `${expected}\n`) resolve();
        else reject(new Error("helper readiness was invalid"));
      }
    };
    const ended = () => {
      cleanup();
      reject(new Error("helper exited before readiness"));
    };
    function cleanup() {
      clearTimeout(timer);
      stream.off("data", consume);
      stream.off("end", ended);
    }
    stream.on("data", consume);
    stream.once("end", ended);
  });
}

function waitForExit(child, timeoutMilliseconds = 3_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("child exit timed out")),
      timeoutMilliseconds,
    );
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function acquireEventually(lockFile, timeoutMilliseconds = 3_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  let lastError;
  while (Date.now() < deadline) {
    try {
      return await acquireLifecycleLock({
        lockFile,
        startupTimeoutMs: 250,
        releaseTimeoutMs: 500,
      });
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  throw lastError ?? new Error("lock acquisition timed out");
}

darwinTest("contends fail closed, releases idempotently, and cannot forge a lease", async (context) => {
  const values = await fixture(context);
  const lockFile = await prepareLock(values);
  const first = await acquireLifecycleLock({ lockFile });
  assert.equal(requireLifecycleLockCapability(first), first);
  assert.throws(
    () => requireLifecycleLockCapability({}),
    /lifecycle lock capability is invalid/,
  );

  await assert.rejects(
    acquireLifecycleLock({ lockFile, startupTimeoutMs: 500 }),
    /lifecycle lock is unavailable/,
  );
  await releaseLifecycleLock(first);
  await releaseLifecycleLock(first);
  assert.throws(
    () => requireLifecycleLockCapability(first),
    /lifecycle lock capability is invalid/,
  );

  const second = await acquireLifecycleLock({ lockFile });
  await releaseLifecycleLock(second);
});

darwinTest("parent SIGKILL closes the holder pipe and releases the kernel lock", async (context) => {
  const values = await fixture(context);
  const lockFile = await prepareLock(values);
  const moduleUrl = pathToFileURL(
    path.join(import.meta.dirname, "macos-lifecycle-lock.mjs"),
  ).href;
  const helperSource = [
    `import { acquireLifecycleLock } from ${JSON.stringify(moduleUrl)};`,
    `globalThis.lease = await acquireLifecycleLock({ lockFile: ${JSON.stringify(lockFile)} });`,
    'process.stdout.write("parent-ready\\n");',
    "setInterval(() => {}, 1_000);",
  ].join("\n");
  const parent = spawn(
    process.execPath,
    ["--input-type=module", "--eval", helperSource],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  context.after(() => {
    if (parent.exitCode === null && parent.signalCode === null) {
      parent.kill("SIGKILL");
    }
  });
  await waitForLine(parent.stdout, "parent-ready");
  await assert.rejects(
    acquireLifecycleLock({ lockFile, startupTimeoutMs: 500 }),
    /lifecycle lock is unavailable/,
  );

  parent.kill("SIGKILL");
  await waitForExit(parent);
  const recovered = await acquireEventually(lockFile);
  await releaseLifecycleLock(recovered);
});

darwinTest("rejects wrong, symlinked, or non-owner-only lock files", async (context) => {
  const values = await fixture(context);
  const expected = path.join(values.evidenceDirectory, LIFECYCLE_LOCK_FILE);

  await mkdir(expected, { recursive: true, mode: 0o700 });
  await assert.rejects(prepareLock(values), /lifecycle lock file is invalid/);
  await rm(expected, { recursive: true, force: true });

  await symlink("/dev/null", expected);
  await assert.rejects(prepareLock(values), /lifecycle lock file is invalid/);
  await rm(expected, { force: true });

  await writeFile(expected, "", { mode: 0o600 });
  await chmod(expected, 0o644);
  await assert.rejects(prepareLock(values), /lifecycle lock file is invalid/);

  await chmod(expected, 0o600);
  const wrong = path.join(values.evidenceDirectory, "wrong.lock");
  await writeFile(wrong, "", { mode: 0o600 });
  await assert.rejects(
    acquireLifecycleLock({ lockFile: wrong }),
    /lifecycle lock file is invalid/,
  );
});

darwinTest("discards bounded child stderr and never leaks filesystem paths", async (context) => {
  const values = await fixture(context);
  const lockFile = await prepareLock(values);
  const fakeTool = path.join(values.root, "fake-lockf");
  await writeFile(
    fakeTool,
    `#!${process.execPath}\nprocess.stderr.write(process.argv.join(" ").repeat(64));\nprocess.exit(75);\n`,
    { mode: 0o700 },
  );
  await chmod(fakeTool, 0o700);

  let failure;
  try {
    await acquireLifecycleLock({
      lockFile,
      testRuntime: { platform: "darwin", lockTool: fakeTool },
      startupTimeoutMs: 500,
    });
  } catch (error) {
    failure = error;
  }
  assert.match(failure?.message ?? "", /lifecycle lock is unavailable/);
  assert.equal(failure.message.includes(values.root), false);
  assert.equal(Buffer.byteLength(failure.message, "utf8") < 128, true);
});

darwinTest("bounds startup and forced release when injected children do not cooperate", async (context) => {
  const values = await fixture(context);
  const lockFile = await prepareLock(values);
  const silentTool = path.join(values.root, "silent-lock-tool");
  await writeFile(
    silentTool,
    `#!${process.execPath}\nprocess.stdin.resume();\nsetInterval(() => {}, 1_000);\n`,
    { mode: 0o700 },
  );
  await chmod(silentTool, 0o700);
  const startupBegan = Date.now();
  await assert.rejects(
    acquireLifecycleLock({
      lockFile,
      testRuntime: { platform: "darwin", lockTool: silentTool },
      startupTimeoutMs: 50,
    }),
    /lifecycle lock is unavailable/,
  );
  assert.equal(Date.now() - startupBegan < 1_000, true);

  const stubbornHolder = path.join(values.root, "stubborn-holder.mjs");
  await writeFile(
    stubbornHolder,
    [
      'process.stdout.write("resume-ir.macos-lifecycle-lock.ready.v1\\n");',
      "process.stdin.resume();",
      "setInterval(() => {}, 1_000);",
    ].join("\n"),
    { mode: 0o600 },
  );
  const capability = await acquireLifecycleLock({
    lockFile,
    testRuntime: { platform: "darwin", holderScript: stubbornHolder },
    releaseTimeoutMs: 50,
  });
  const releaseBegan = Date.now();
  await releaseLifecycleLock(capability);
  assert.equal(Date.now() - releaseBegan < 1_000, true);

  const recovered = await acquireEventually(lockFile);
  await releaseLifecycleLock(recovered);
});
