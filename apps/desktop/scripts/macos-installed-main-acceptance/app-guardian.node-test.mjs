import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  APP_GUARDIAN_FILE,
  guardianCommitLine,
  parseGuardianHandshake,
} from "./app-guardian.mjs";

function waitForExit(child, timeoutMs = 5_000) {
  return new Promise((resolve, reject) => {
    if (child.exitCode !== null || child.signalCode !== null) return resolve();
    const timer = setTimeout(() => reject(new Error("exit timeout")), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

function waitForLine(stream, timeoutMs = 3_000) {
  return new Promise((resolve, reject) => {
    let source = "";
    const timer = setTimeout(() => reject(new Error("line timeout")), timeoutMs);
    stream.on("data", (chunk) => {
      source += chunk.toString("utf8");
      const newline = source.indexOf("\n");
      if (newline >= 0) {
        clearTimeout(timer);
        resolve(source.slice(0, newline + 1));
      }
    });
  });
}

async function processIdentity(pid) {
  const child = spawn(
    "/bin/ps",
    ["-p", String(pid), "-o", "pid=,ppid=,pgid=,command="],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  let stdout = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString("utf8");
  });
  await waitForExit(child);
  return stdout.trim();
}

async function directChildren(pid) {
  const child = spawn(
    "/bin/ps",
    ["-axo", "pid=,ppid=,command="],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  let stdout = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString("utf8");
  });
  await waitForExit(child);
  return stdout
    .split("\n")
    .map((line) => line.match(/^\s*(\d+)\s+(\d+)\s+(.+)$/))
    .filter((match) => match && Number(match[2]) === pid)
    .map((match) => Number(match[1]));
}

async function processGroup(pgid) {
  const child = spawn(
    "/bin/ps",
    ["-axo", "pid=,pgid="],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  let stdout = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString("utf8");
  });
  await waitForExit(child);
  return stdout
    .split("\n")
    .map((line) => line.match(/^\s*(\d+)\s+(\d+)$/))
    .filter((match) => match && Number(match[2]) === pgid)
    .map((match) => Number(match[1]));
}

test("guardian waits for durable commit, keeps App in its authority PGID, and cleans on parent EOF", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-guardian-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const app = path.join(root, "synthetic-app");
  await writeFile(
    app,
    "#!/bin/sh\ntrap 'exit 0' TERM INT\nwhile :; do /bin/sleep 1; done\n",
  );
  await chmod(app, 0o700);
  const authority = "8".repeat(64);
  const guardian = spawn(
    process.execPath,
    [
      APP_GUARDIAN_FILE,
      "--session-authority",
      authority,
      "--desktop-executable",
      app,
      "--home",
      root,
    ],
    { detached: true, stdio: ["pipe", "pipe", "pipe"] },
  );
  context.after(() => {
    try {
      process.kill(-guardian.pid, "SIGKILL");
    } catch {}
  });

  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.deepEqual(await directChildren(guardian.pid), []);
  guardian.stdin.write(guardianCommitLine(authority));
  const handshake = parseGuardianHandshake(
    await waitForLine(guardian.stdout),
    authority,
  );
  const appIdentity = (await processIdentity(handshake.appPid)).split(/\s+/, 4);
  assert.equal(Number(appIdentity[1]), guardian.pid);
  assert.equal(Number(appIdentity[2]), guardian.pid);

  guardian.stdin.end();
  await waitForExit(guardian);
  assert.equal(await processIdentity(handshake.appPid), "");
});

test("parent EOF removes an App that ignores TERM and every inherited descendant", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-guardian-tree-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const app = path.join(root, "synthetic-app-tree");
  await writeFile(
    app,
    "#!/bin/sh\ntrap '' TERM HUP INT\n/bin/sh -c 'trap \"\" TERM HUP INT; while :; do /bin/sleep 1; done' &\nwhile :; do /bin/sleep 1; done\n",
  );
  await chmod(app, 0o700);
  const authority = "7".repeat(64);
  const guardian = spawn(
    process.execPath,
    [
      APP_GUARDIAN_FILE,
      "--session-authority",
      authority,
      "--desktop-executable",
      app,
      "--home",
      root,
    ],
    { detached: true, stdio: ["pipe", "pipe", "pipe"] },
  );
  context.after(() => {
    try {
      process.kill(-guardian.pid, "SIGKILL");
    } catch {}
  });
  guardian.stdin.write(guardianCommitLine(authority));
  await waitForLine(guardian.stdout);
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if ((await processGroup(guardian.pid)).length >= 3) break;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.ok((await processGroup(guardian.pid)).length >= 3);

  guardian.stdin.end();
  await waitForExit(guardian);
  await new Promise((resolve) => setTimeout(resolve, 100));
  const group = await processIdentity(guardian.pid);
  assert.equal(group, "");
  assert.deepEqual(await processGroup(guardian.pid), []);
});
