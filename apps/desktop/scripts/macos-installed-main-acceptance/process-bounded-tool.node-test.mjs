import assert from "node:assert/strict";
import { spawn as spawnChild } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";

import { runAcceptanceCli } from "../macos-installed-main-acceptance.mjs";
import {
  APP_GUARDIAN_FILE,
  authorityAnchorCommand,
  guardianCommitLine,
  parseGuardianHandshake,
} from "./app-guardian.mjs";
import { runBoundedTool } from "./bounded-process.mjs";
import {
  ACCEPTANCE_SCHEMA,
  APP_EXECUTABLE,
  DAEMON_EXECUTABLE,
  DEFAULT_INSTALLED_EXECUTABLES,
  EMBEDDING_EXECUTABLE,
  PDF_RENDER_EXECUTABLE,
  WORKSPACE_MARKER,
  WORKSPACE_MARKER_SCHEMA,
  WORKSPACE_PREFIX,
} from "./core.mjs";
import {
  persistWorkspaceMarker,
  readWorkspaceMarker,
  updateWorkspaceMarker,
  workspaceMarker,
} from "./filesystem-cow.mjs";
import {
  assertNoInstalledRuntime,
  installedAcceptanceEnvironment,
  launchInstalledApp,
  quitInstalledApp,
  recoverStaleWorkspaceRuntime,
  selectSessionApplication,
} from "./process-lifecycle.mjs";
import { readProcessStartTime } from "./process-identity.mjs";

function waitForChildExit(child, timeoutMs = 5_000) {
  return new Promise((resolve, reject) => {
    if (child.exitCode !== null || child.signalCode !== null) return resolve();
    const timer = setTimeout(() => reject(new Error("exit timeout")), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

function waitForGuardianLine(stream, timeoutMs = 3_000) {
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

async function liveProcessGroup(pgid) {
  const result = await runBoundedTool(
    "/bin/ps",
    ["-axo", "pid=,pgid="],
    { timeoutMs: 5_000 },
  );
  return result.stdout
    .split("\n")
    .map((line) => line.match(/^\s*(\d+)\s+(\d+)$/))
    .filter((match) => match && Number(match[2]) === pgid)
    .map((match) => Number(match[1]));
}

test("bounded tools always use argv execution with shell disabled", async () => {
  let captured;
  const spawnTool = (command, args, toolOptions) => {
    captured = { command, args, toolOptions };
    const child = new EventEmitter();
    child.pid = 9_999;
    child.exitCode = null;
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.kill = () => {};
    queueMicrotask(() => {
      child.stdout.end("bounded");
      child.stderr.end();
      child.exitCode = 0;
      child.emit("exit", 0, null);
    });
    return child;
  };
  const result = await runBoundedTool("/synthetic/tool", ["one", "two"], {
    spawnTool,
    timeoutMs: 1_000,
  });
  assert.equal(result.status, 0);
  assert.equal(result.stdout, "bounded");
  assert.equal(captured.toolOptions.shell, false);
  assert.deepEqual(captured.args, ["one", "two"]);
  assert.equal(typeof captured.command, "string");
});

test("an aborted bounded helper is terminated by its exact spawned process group", async () => {
  const controller = new AbortController();
  const signals = [];
  const child = new EventEmitter();
  child.pid = 7_777;
  child.exitCode = null;
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.kill = (signal) => {
    signals.push(signal);
    child.exitCode = null;
    queueMicrotask(() => child.emit("exit", null, signal));
  };
  const running = runBoundedTool("/synthetic/tool", [], {
    killProcess: (pid, signal) => {
      assert.equal(pid, -child.pid);
      signals.push(signal);
      child.exitCode = null;
      queueMicrotask(() => child.emit("exit", null, signal));
    },
    signal: controller.signal,
    spawnTool: () => child,
    timeoutMs: 1_000,
  });
  controller.abort();
  await assert.rejects(running, /acceptance_interrupted/);
  assert.deepEqual(signals, ["SIGTERM"]);
});

test("a marker-write failure terminates the already-spawned helper group", async () => {
  const signals = [];
  const child = new EventEmitter();
  child.pid = 7_778;
  child.exitCode = null;
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.kill = () => {};
  await assert.rejects(
    runBoundedTool("/synthetic/tool", [], {
      killProcess: (pid, signal) => {
        signals.push([pid, signal]);
        queueMicrotask(() => child.emit("exit", null, signal));
      },
      onSpawn: async () => {
        throw new Error("synthetic marker failure");
      },
      spawnTool: () => child,
      timeoutMs: 1_000,
    }),
    /synthetic marker failure/,
  );
  assert.deepEqual(signals, [[-7_778, "SIGTERM"]]);
});

test("global residue inspection rejects the exact installed runtime", async () => {
  const result = (stdout) => ({
    status: 0,
    stdout,
    stderr: "",
    timedOut: false,
    overflow: false,
  });
  await assertNoInstalledRuntime(async () =>
    result("101 1 101 /Applications/Other.app/Contents/MacOS/other\n"),
  );
  await assert.rejects(
    assertNoInstalledRuntime(async () =>
      result(`202 1 202 ${APP_EXECUTABLE}\n`),
    ),
    /installed_runtime_already_running/,
  );
  for (const executable of [
    DAEMON_EXECUTABLE,
    EMBEDDING_EXECUTABLE,
    PDF_RENDER_EXECUTABLE,
  ]) {
    await assert.rejects(
      assertNoInstalledRuntime(async () =>
        result(`303 1 999 ${executable} --resident\n`),
      ),
      /installed_runtime_already_running/,
    );
  }
});

test("installed launch environment is a closed canonical allowlist", () => {
  const env = installedAcceptanceEnvironment("/synthetic/acceptance-home");
  assert.deepEqual(env, {
    HOME: "/synthetic/acceptance-home",
    LANG: "C",
    LC_ALL: "C",
    PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
    TMPDIR: "/tmp",
  });
  for (const forbidden of [
    "RESUME_IR_DATA_DIR",
    "RESUME_IR_DAEMON_BINARY",
    "RESUME_IR_EMBEDDING_COMMAND",
    "TAURI_DEBUG",
    "VITE_TEST",
    "RUST_LOG",
    "DYLD_INSERT_LIBRARIES",
  ]) {
    assert.equal(Object.hasOwn(env, forbidden), false);
  }

});

test("an active-marker fsync failure makes the guardian terminate the exact inherited App group", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-launch-failpoint-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const dataDir = path.join(
    root,
    "Library",
    "Application Support",
    "local.resume-ir.desktop",
  );
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  await chmod(path.join(root, "Library"), 0o700);
  await chmod(path.join(root, "Library", "Application Support"), 0o700);
  const runId = "6".repeat(64);
  await persistWorkspaceMarker(root, workspaceMarker(runId));
  const executablePaths = {
    desktop: "/usr/bin/yes",
    daemon: "/synthetic/daemon",
    embedding_runtime: "/synthetic/embedding",
    pdf_renderer: "/synthetic/pdf-renderer",
  };
  let markerWrites = 0;
  await assert.rejects(
    launchInstalledApp(
      { dataDir, home: root, root, runId },
      executablePaths,
      {
        runTool: runBoundedTool,
        persistMarker: async (...args) => {
          markerWrites += 1;
          if (markerWrites === 3) {
            throw new Error("synthetic active marker fsync failure");
          }
          return updateWorkspaceMarker(...args);
        },
      },
    ),
    /synthetic active marker fsync failure/,
  );
  assert.equal(markerWrites, 4);
  assert.equal((await readWorkspaceMarker(root)).state, "app_stopped");
  await assertNoInstalledRuntime(runBoundedTool, executablePaths);
});

test("pending-launch recovery cleans an inherited App after the guardian is SIGKILLed", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-pending-recovery-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const dataDir = path.join(
    root,
    "Library",
    "Application Support",
    "local.resume-ir.desktop",
  );
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  await chmod(path.join(root, "Library"), 0o700);
  await chmod(path.join(root, "Library", "Application Support"), 0o700);
  const runId = "5".repeat(64);
  const authority = "4".repeat(64);
  await persistWorkspaceMarker(root, workspaceMarker(runId));
  const executablePaths = {
    desktop: "/usr/bin/yes",
    daemon: "/synthetic/daemon",
    embedding_runtime: "/synthetic/embedding",
    pdf_renderer: "/synthetic/pdf-renderer",
  };
  await updateWorkspaceMarker(root, runId, {
    state: "launch_intent",
    session_authority: authority,
    guardian: null,
    authority_anchor: null,
    application: null,
  });
  const guardian = spawnChild(
    process.execPath,
    [
      APP_GUARDIAN_FILE,
      "--session-authority",
      authority,
      "--desktop-executable",
      executablePaths.desktop,
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
  const guardianRecord = {
    pid: guardian.pid,
    pgid: guardian.pid,
    start_time: await readProcessStartTime(guardian.pid, runBoundedTool),
    executable: process.execPath,
    session_authority: authority,
  };
  await updateWorkspaceMarker(root, runId, {
    state: "launch_pending",
    session_authority: authority,
    guardian: guardianRecord,
    application: null,
  });
  guardian.stdin.write(guardianCommitLine(authority));
  const handshake = parseGuardianHandshake(
    await waitForGuardianLine(guardian.stdout),
    authority,
  );
  process.kill(guardian.pid, "SIGKILL");
  await waitForChildExit(guardian);

  await recoverStaleWorkspaceRuntime(
    {
      dataDir,
      home: root,
      marker: await readWorkspaceMarker(root),
      root,
    },
    runBoundedTool,
    { executablePaths },
  );
  await assertNoInstalledRuntime(runBoundedTool, executablePaths);
  assert.equal(
    process.platform === "win32" ? false : handshake.appPid > 1,
    true,
  );
});

test("stale recovery cleans authority-group descendants after both guardian and App disappear", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-orphan-recovery-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const dataDir = path.join(
    root,
    "Library",
    "Application Support",
    "local.resume-ir.desktop",
  );
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  await chmod(path.join(root, "Library"), 0o700);
  await chmod(path.join(root, "Library", "Application Support"), 0o700);
  const app = path.join(root, "synthetic-orphan-app");
  await writeFile(
    app,
    "#!/bin/sh\ntrap '' TERM HUP INT\n/usr/bin/yes >/dev/null &\nwhile :; do /bin/sleep 1; done\n",
  );
  await chmod(app, 0o700);
  const runId = "3".repeat(64);
  const authority = "2".repeat(64);
  await persistWorkspaceMarker(root, workspaceMarker(runId));
  await updateWorkspaceMarker(root, runId, {
    state: "launch_intent",
    session_authority: authority,
    guardian: null,
    application: null,
  });
  const executablePaths = {
    desktop: app,
    daemon: "/synthetic/daemon",
    embedding_runtime: "/usr/bin/yes",
    pdf_renderer: "/bin/sleep",
  };
  const guardian = spawnChild(
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
  const guardianRecord = {
    pid: guardian.pid,
    pgid: guardian.pid,
    start_time: await readProcessStartTime(guardian.pid, runBoundedTool),
    executable: process.execPath,
    session_authority: authority,
  };
  await updateWorkspaceMarker(root, runId, {
    state: "launch_pending",
    session_authority: authority,
    guardian: guardianRecord,
    application: null,
  });
  guardian.stdin.write(guardianCommitLine(authority));
  const handshake = parseGuardianHandshake(
    await waitForGuardianLine(guardian.stdout),
    authority,
  );
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if ((await liveProcessGroup(guardian.pid)).length >= 3) break;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.ok((await liveProcessGroup(guardian.pid)).length >= 3);
  process.kill(guardian.pid, "SIGKILL");
  await waitForChildExit(guardian);
  process.kill(handshake.appPid, "SIGKILL");
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.ok((await liveProcessGroup(guardian.pid)).length >= 1);

  await recoverStaleWorkspaceRuntime(
    {
      dataDir,
      home: root,
      marker: await readWorkspaceMarker(root),
      root,
    },
    runBoundedTool,
    { executablePaths },
  );
  assert.deepEqual(await liveProcessGroup(guardian.pid), []);
});

test("targeted normal quit rejects wrong identity and never broadcasts by bundle id", async () => {
  const startTime = "Mon Jul 20 12:34:56 2026";
  const authority = "9".repeat(64);
  const appCommand = `${APP_EXECUTABLE} --resume-ir-acceptance-session-authority=${authority}`;
  const target = {
    pid: 4_001,
    pgid: 3_999,
    executablePaths: DEFAULT_INSTALLED_EXECUTABLES,
    sessionAuthority: authority,
    applicationRecord: {
      pid: 4_001,
      pgid: 3_999,
      start_time: startTime,
      executable: APP_EXECUTABLE,
      session_authority: authority,
    },
    child: { pid: 3_999, stdin: { end() {} } },
    monitor: { promise: Promise.resolve(), settled: true },
  };
  assert.throws(
    () =>
      selectSessionApplication(
        [
          {
            pid: target.pid,
            ppid: 1,
            pgid: 9_999,
            command: appCommand,
          },
        ],
        target,
      ),
    /normal_quit_identity_changed/,
  );
  assert.throws(
    () =>
      selectSessionApplication(
        [
          {
            pid: 4_002,
            ppid: 1,
            pgid: 4_002,
            command: appCommand,
          },
        ],
        target,
      ),
    /normal_quit_identity_changed/,
  );

  const concurrent = [
    {
      pid: target.pid,
      ppid: 1,
      pgid: target.pgid,
      command: appCommand,
    },
    {
      pid: 4_002,
      ppid: 1,
      pgid: 4_002,
      command: appCommand,
    },
  ];
  assert.equal(selectSessionApplication(concurrent, target).pid, target.pid);

  const calls = [];
  let processReads = 0;
  const result = (stdout) => ({
    status: 0,
    stdout,
    stderr: "",
    timedOut: false,
    overflow: false,
  });
  await quitInstalledApp(
    { ...target, stopped: false },
    async (command, args) => {
      calls.push([command, args]);
      if (command === "/bin/ps") {
        if (args.includes("lstart=")) return result(`${startTime}\n`);
        processReads += 1;
        return result(
          processReads === 1
            ? `${target.pid} 1 ${target.pgid} ${appCommand}\n`
            : "",
        );
      }
      assert.equal(command, "/usr/bin/osascript");
      return result("resume-ir.targeted-terminate.accepted.v1\n");
    },
  );
  const invocation = calls.find(([command]) => command === "/usr/bin/osascript");
  assert.deepEqual(invocation[1].slice(0, 2), ["-l", "JavaScript"]);
  assert.equal(invocation[1].at(-2), String(target.pid));
  assert.equal(invocation[1].at(-1), APP_EXECUTABLE);
  assert.equal(invocation[1].some((value) => value.includes("local.resume")), false);
  const script = await readFile(invocation[1][2], "utf8");
  assert.match(script, /runningApplicationWithProcessIdentifier/);
  assert.doesNotMatch(script, /runningApplicationsWithBundleIdentifier/);
  assert.doesNotMatch(script, /local\.resume-ir\.desktop/);
});

test("stale recovery targets only marker-bound clone helpers and detached App PIDs", async (context) => {
  const startTime = "Mon Jul 20 12:34:56 2026";
  const temporaryParent = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-stale-runtime-"),
  );
  context.after(() => rm(temporaryParent, { recursive: true, force: true }));
  await chmod(temporaryParent, 0o700);

  const writeMarker = async (suffix, marker) => {
    const root = path.join(temporaryParent, `${WORKSPACE_PREFIX}${suffix}`);
    const dataDir = path.join(
      root,
      "Library",
      "Application Support",
      "local.resume-ir.desktop",
    );
    await mkdir(dataDir, { recursive: true, mode: 0o700 });
    await chmod(root, 0o700);
    await chmod(path.join(root, "Library"), 0o700);
    await chmod(path.join(root, "Library", "Application Support"), 0o700);
    await writeFile(
      path.join(root, WORKSPACE_MARKER),
      `${JSON.stringify(marker)}\n`,
      { mode: 0o600 },
    );
    return { dataDir, home: root, marker, root };
  };
  const clone = await writeMarker("clone", {
    schema_version: WORKSPACE_MARKER_SCHEMA,
    acceptance_schema: ACCEPTANCE_SCHEMA,
    run_id: "1".repeat(64),
    state: "clone_active",
    session_authority: null,
    helper: {
      kind: "cow_clone",
      pid: 8_101,
      pgid: 8_101,
      start_time: startTime,
      executable: process.execPath,
      session_authority: "1".repeat(64),
      lock_kind: null,
    },
    guardian: null,
    authority_anchor: null,
    application: null,
  });
  let cloneAlive = true;
  const killed = [];
  await recoverStaleWorkspaceRuntime(clone, async (_command, args) => ({
    status: 0,
    stdout: args.includes("lstart=")
      ? `${startTime}\n`
      : cloneAlive
        ? `8101 7000 8101 ${process.execPath} /synthetic/macos-installed-main-acceptance.mjs --internal-cow-clone /private/source ${clone.dataDir}\n`
        : "",
    stderr: "",
    timedOut: false,
    overflow: false,
  }), {
    entryScriptFile: "/synthetic/macos-installed-main-acceptance.mjs",
    killProcess: (pid, signal) => {
      killed.push([pid, signal]);
      cloneAlive = false;
    },
  });
  assert.deepEqual(killed, [[-8_101, "SIGTERM"]]);

  const app = await writeMarker("app", {
    schema_version: WORKSPACE_MARKER_SCHEMA,
    acceptance_schema: ACCEPTANCE_SCHEMA,
    run_id: "2".repeat(64),
    state: "app_running",
    session_authority: "2".repeat(64),
    helper: null,
    guardian: {
      pid: 8_200,
      pgid: 8_200,
      start_time: startTime,
      executable: process.execPath,
      session_authority: "2".repeat(64),
    },
    authority_anchor: {
      pid: 8_203,
      pgid: 8_200,
      start_time: startTime,
      executable: process.execPath,
      session_authority: "2".repeat(64),
    },
    application: {
      pid: 8_201,
      pgid: 8_200,
      start_time: startTime,
      executable: APP_EXECUTABLE,
      session_authority: "2".repeat(64),
    },
  });
  let appAlive = true;
  const appKills = [];
  const guardianCommand = `${process.execPath} ${APP_GUARDIAN_FILE} --session-authority ${"2".repeat(64)} --desktop-executable ${APP_EXECUTABLE} --home ${app.home}`;
  const appRunner = async (_command, args) => ({
    status: 0,
    stdout: args.includes("lstart=")
      ? `${startTime}\n`
      : appAlive
        ? [
            `8200 1 8200 ${guardianCommand}`,
            `8201 8200 8200 ${APP_EXECUTABLE} --resume-ir-acceptance-session-authority=${"2".repeat(64)}`,
            `8202 8201 8200 ${DAEMON_EXECUTABLE} --data-dir ${app.dataDir}`,
            `8203 8200 8200 ${authorityAnchorCommand("2".repeat(64))}`,
            "9300 1 9300 /Applications/Other.app/Contents/MacOS/other",
            "",
          ].join("\n")
        : "",
    stderr: "",
    timedOut: false,
    overflow: false,
  });
  await recoverStaleWorkspaceRuntime(app, appRunner, {
    killProcess: (pid, signal) => {
      appKills.push([pid, signal]);
      appAlive = false;
    },
  });
  assert.deepEqual(appKills, [[-8_200, "SIGTERM"]]);

  const reusedKills = [];
  await assert.rejects(
    recoverStaleWorkspaceRuntime(
      app,
      async (_command, args) => ({
        status: 0,
        stdout: args.includes("lstart=")
          ? "Tue Jul 21 12:34:56 2026\n"
          : [
              `8200 1 8200 ${guardianCommand}`,
              `8201 8200 8200 ${APP_EXECUTABLE} --resume-ir-acceptance-session-authority=${"2".repeat(64)}`,
              `8203 8200 8200 ${authorityAnchorCommand("2".repeat(64))}`,
              "",
            ].join("\n"),
        stderr: "",
        timedOut: false,
        overflow: false,
      }),
      { killProcess: (...args) => reusedKills.push(args) },
    ),
    /stale_workspace_recovery_failed/,
  );
  assert.deepEqual(reusedKills, []);
});

test("SIGINT and SIGTERM return nonzero only after the acceptance cleanup promise settles", async () => {
  for (const signalName of ["SIGINT", "SIGTERM"]) {
    const signalSource = new EventEmitter();
    const order = [];
    const writes = [];
    const exitCode = await runAcceptanceCli({
      argv: [
        "--authorized-source-data-dir",
        "/synthetic/private/source",
      ],
      runAcceptance: async (_options, { signal }) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => {
            setImmediate(() => {
              order.push("cleanup");
              reject(signal.reason);
            });
          });
          queueMicrotask(() => signalSource.emit(signalName));
        }),
      signalSource,
      write: (value) => {
        order.push("write");
        writes.push(value);
      },
    });
    assert.equal(exitCode, 1);
    assert.deepEqual(order, ["cleanup", "write"]);
    assert.match(writes.join(""), /"error_code":"acceptance_interrupted"/);
  }
});
