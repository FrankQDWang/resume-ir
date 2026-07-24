import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { toolSucceeded } from "./bounded-process.mjs";
import {
  APP_GUARDIAN_FILE,
  authorityAnchorCommand,
  guardianCommitLine,
  parseGuardianHandshake,
} from "./app-guardian.mjs";
import {
  APP_DATA_DIRECTORY,
  APP_EXECUTABLE,
  DAEMON_EXECUTABLE,
  DEFAULT_INSTALLED_EXECUTABLES,
  DIGEST,
  ENTRY_SCRIPT_FILE,
  MAX_TOOL_OUTPUT_BYTES,
  POLL_MS,
  QUIT_TIMEOUT_MS,
  READY_TIMEOUT_MS,
  TOOL_TIMEOUT_MS,
  AcceptanceError,
  createExitMonitor,
  fail,
  signalProcessGroup,
  throwIfAborted,
  wait,
  waitBounded,
} from "./core.mjs";
import {
  FLOCK_HOLDER_FILE,
  updateWorkspaceMarker,
  validateWorkspaceMarker,
} from "./filesystem-cow.mjs";
import {
  readDaemonConnection,
  readyStatus,
  requestJson,
} from "./ipc-contracts.mjs";
import {
  readProcessStartTime,
  requireRecordedProcessStart,
} from "./process-identity.mjs";

const QUIT_INSTALLED_APP_SCRIPT = fileURLToPath(
  new URL("./quit-installed-app.jxa", import.meta.url),
);
const TARGETED_TERMINATE_ACCEPTED =
  "resume-ir.targeted-terminate.accepted.v1\n";

export function parseProcessTable(source) {
  const processes = [];
  for (const line of source.split("\n")) {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.+)$/);
    if (!match) continue;
    processes.push({
      pid: Number(match[1]),
      ppid: Number(match[2]),
      pgid: Number(match[3]),
      command: match[4],
    });
  }
  return processes;
}

async function processTable(runTool) {
  const result = await runTool(
    "/bin/ps",
    ["-ww", "-axo", "pid=,ppid=,pgid=,command="],
    { timeoutMs: TOOL_TIMEOUT_MS },
  );
  if (!toolSucceeded(result)) fail("process_inspection_failed");
  return parseProcessTable(result.stdout);
}

export function exactExecutableCommand(command, executable) {
  return command === executable || command.startsWith(`${executable} `);
}

function exactApplicationCommand(command, executable, authority) {
  const expected =
    `${executable} --resume-ir-acceptance-session-authority=${authority}`;
  return command === expected || command.startsWith(`${expected}\\012`);
}

function executableList(executablePaths) {
  const values = Object.values(executablePaths ?? {});
  if (
    values.length !== 4 ||
    new Set(values).size !== 4 ||
    !values.every((value) => typeof value === "string" && path.isAbsolute(value))
  ) {
    fail("installed_composition_invalid");
  }
  return values;
}

export async function installedRuntimeProcesses(
  runTool,
  executablePaths = DEFAULT_INSTALLED_EXECUTABLES,
) {
  const processes = await processTable(runTool);
  const executables = executableList(executablePaths);
  return processes.filter(({ command }) =>
    executables.some((executable) => exactExecutableCommand(command, executable)),
  );
}

export async function assertNoInstalledRuntime(
  runTool,
  executablePaths = DEFAULT_INSTALLED_EXECUTABLES,
) {
  if ((await installedRuntimeProcesses(runTool, executablePaths)).length !== 0) {
    fail("installed_runtime_already_running");
  }
}

export function installedAcceptanceEnvironment(home) {
  if (typeof home !== "string" || !path.isAbsolute(home) || home.includes("\0")) {
    fail("workspace_invalid");
  }
  return Object.freeze({
    HOME: home,
    LANG: "C",
    LC_ALL: "C",
    PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
    TMPDIR: "/tmp",
  });
}

function guardianHandshake(child, monitor, authority) {
  return new Promise((resolve, reject) => {
    let source = "";
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      callback(value);
    };
    const timer = setTimeout(
      () => finish(reject, new AcceptanceError("installed_app_launch_failed")),
      5_000,
    );
    child.stdout?.on("data", (chunk) => {
      source += chunk.toString("utf8");
      if (Buffer.byteLength(source, "utf8") > 1_024) {
        finish(reject, new AcceptanceError("installed_app_launch_failed"));
        return;
      }
      const newline = source.indexOf("\n");
      if (newline >= 0) {
        try {
          finish(
            resolve,
            parseGuardianHandshake(source.slice(0, newline + 1), authority),
          );
        } catch (error) {
          finish(reject, error);
        }
      }
    });
    child.stdout?.on("error", () =>
      finish(reject, new AcceptanceError("installed_app_launch_failed")),
    );
    monitor.promise.then(() =>
      finish(reject, new AcceptanceError("installed_app_launch_failed")),
    );
  });
}

async function terminateGuardian(child, monitor) {
  child.stdin?.end();
  if (await waitBounded(monitor.promise, 2_000)) return;
  signalProcessGroup(child, "SIGTERM");
  if (await waitBounded(monitor.promise, 1_000)) return;
  signalProcessGroup(child, "SIGKILL");
  await waitBounded(monitor.promise, 1_000);
}

export async function launchInstalledApp(
  workspace,
  executablePaths = DEFAULT_INSTALLED_EXECUTABLES,
  {
    persistMarker = updateWorkspaceMarker,
    runTool,
    spawnTool = spawn,
  } = {},
) {
  const dataDir = path.join(
    workspace.home,
    "Library",
    "Application Support",
    APP_DATA_DIRECTORY,
  );
  if (dataDir !== workspace.dataDir) fail("workspace_invalid");
  if (typeof runTool !== "function") fail("installed_app_launch_failed");
  const authority = randomBytes(32).toString("hex");
  await persistMarker(workspace.root, workspace.runId, {
    state: "launch_intent",
    session_authority: authority,
    guardian: null,
    authority_anchor: null,
    application: null,
  });
  let child;
  try {
    child = spawnTool(process.execPath, [
      APP_GUARDIAN_FILE,
      "--session-authority",
      authority,
      "--desktop-executable",
      executablePaths.desktop,
      "--home",
      workspace.home,
    ], {
      cwd: "/",
      detached: true,
      env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin" },
      shell: false,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
  } catch {
    fail("installed_app_launch_failed");
  }
  const monitor = createExitMonitor(child);
  let stderrBytes = 0;
  child.stderr?.on("data", (chunk) => {
    stderrBytes = Math.min(
      MAX_TOOL_OUTPUT_BYTES + 1,
      stderrBytes + chunk.length,
    );
  });
  child.stderr?.on("error", () => {});
  child.stdin?.on("error", () => {});
  try {
    const guardianStartTime = await readProcessStartTime(child.pid, runTool);
    const guardianRecord = {
      pid: child.pid,
      pgid: child.pid,
      start_time: guardianStartTime,
      executable: process.execPath,
      session_authority: authority,
    };
    await persistMarker(workspace.root, workspace.runId, {
      state: "launch_pending",
      session_authority: authority,
      guardian: guardianRecord,
      authority_anchor: null,
      application: null,
    });
    const handshakePromise = guardianHandshake(child, monitor, authority);
    child.stdin.write(guardianCommitLine(authority));
    const handshake = await handshakePromise;
    let appProcess;
    let anchorProcess;
    for (let attempt = 0; attempt < 10; attempt += 1) {
      const processes = await processTable(runTool);
      appProcess = processes.find(({ pid }) => pid === handshake.appPid);
      anchorProcess = processes.find(({ pid }) => pid === handshake.anchorPid);
      if (appProcess && anchorProcess) break;
      await wait(POLL_MS);
    }
    if (
      !appProcess ||
      !anchorProcess ||
      appProcess.ppid !== child.pid ||
      appProcess.pgid !== child.pid ||
      anchorProcess.ppid !== child.pid ||
      anchorProcess.pgid !== child.pid ||
      anchorProcess.command !== authorityAnchorCommand(authority) ||
      !exactApplicationCommand(
        appProcess.command,
        executablePaths.desktop,
        authority,
      )
    ) {
      fail("installed_app_launch_failed");
    }
    const appStartTime = await readProcessStartTime(handshake.appPid, runTool);
    const anchorStartTime = await readProcessStartTime(
      handshake.anchorPid,
      runTool,
    );
    const anchorRecord = {
      pid: handshake.anchorPid,
      pgid: child.pid,
      start_time: anchorStartTime,
      executable: process.execPath,
      session_authority: authority,
    };
    const applicationRecord = {
      pid: handshake.appPid,
      pgid: child.pid,
      start_time: appStartTime,
      executable: executablePaths.desktop,
      session_authority: authority,
    };
    await persistMarker(workspace.root, workspace.runId, {
      state: "app_running",
      session_authority: authority,
      guardian: guardianRecord,
      authority_anchor: anchorRecord,
      application: applicationRecord,
    });
    return {
      applicationRecord,
      anchorRecord,
      child,
      dataDir,
      executablePaths,
      guardianRecord,
      home: workspace.home,
      instanceId: null,
      launchId: null,
      monitor,
      pgid: child.pid,
      pid: handshake.appPid,
      sessionAuthority: authority,
      startTime: appStartTime,
      stderrOverflow: () => stderrBytes > MAX_TOOL_OUTPUT_BYTES,
      stopped: false,
      trackedDaemonPid: null,
    };
  } catch (error) {
    await terminateGuardian(child, monitor);
    await persistMarker(workspace.root, workspace.runId, {
      state: "app_stopped",
      session_authority: null,
      guardian: null,
      authority_anchor: null,
      application: null,
    }).catch(() => {});
    throw error;
  }
}

export async function pollStatus(
  session,
  predicate,
  timeoutMs,
  expectedInstance,
  signal,
  runTool,
) {
  const deadline = Date.now() + timeoutMs;
  let instanceId = expectedInstance ?? session.instanceId;
  let launchId = session.launchId;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    if (session.monitor.settled || session.stderrOverflow()) {
      fail("installed_app_exited");
    }
    try {
      const connection = await readDaemonConnection(session.dataDir);
      if (!(await connectionBelongsToOwnedDaemon(session, connection, runTool))) {
        await wait(POLL_MS);
        continue;
      }
      if (instanceId === null) instanceId = connection.instanceId;
      if (launchId === null) launchId = connection.launchId;
      if (connection.instanceId !== instanceId || connection.launchId !== launchId) {
        fail("daemon_restarted_unexpectedly");
      }
      session.instanceId = instanceId;
      session.launchId = launchId;
      const status = await requestJson(
        connection.urls.status,
        connection.token,
        undefined,
        signal,
      );
      if (predicate(status)) return { connection, instanceId, launchId, status };
    } catch (error) {
      if (
        error instanceof AcceptanceError &&
        [
          "daemon_process_ambiguous",
          "daemon_restarted_unexpectedly",
          "installed_app_exited",
          "process_inspection_failed",
        ].includes(error.code)
      ) {
        throw error;
      }
    }
    await wait(POLL_MS);
  }
  fail("daemon_status_timeout");
}

export async function waitForNewGenerationReady(
  session,
  oldInstanceId,
  signal,
  runTool,
) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  const oldLaunchId = session.launchId;
  if (!DIGEST.test(oldLaunchId ?? "")) fail("daemon_generation_invalid");
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    if (session.monitor.settled || session.stderrOverflow()) {
      fail("installed_app_exited");
    }
    try {
      const connection = await readDaemonConnection(session.dataDir);
      if (
        connection.instanceId !== oldInstanceId &&
        connection.launchId !== oldLaunchId &&
        (await connectionBelongsToOwnedDaemon(session, connection, runTool))
      ) {
        const status = await requestJson(
          connection.urls.status,
          connection.token,
          undefined,
          signal,
        );
        if (readyStatus(status)) {
          session.instanceId = connection.instanceId;
          session.launchId = connection.launchId;
          return {
            connection,
            instanceId: connection.instanceId,
            launchId: connection.launchId,
            status,
          };
        }
      }
    } catch (error) {
      if (
        error instanceof AcceptanceError &&
        ["daemon_process_ambiguous", "process_inspection_failed"].includes(
          error.code,
        )
      ) {
        throw error;
      }
      // Endpoint rotation is expected after the exact strong kill.
    }
    await wait(POLL_MS);
  }
  fail("daemon_recovery_timeout");
}

export async function connectionBelongsToOwnedDaemon(
  session,
  connection,
  runTool,
) {
  if (typeof runTool !== "function") fail("process_inspection_failed");
  const candidates = (await processTable(runTool)).filter(
    ({ ppid, pgid, command }) =>
      ppid === session.pid &&
      pgid === session.pgid &&
      exactExecutableCommand(command, session.executablePaths.daemon) &&
      command.includes("--data-dir") &&
      command.includes(session.dataDir) &&
      command.includes("--launch-id") &&
      command.includes(connection.launchId),
  );
  if (candidates.length > 1) fail("daemon_process_ambiguous");
  return candidates.length === 1;
}

export async function groupProcesses(session, runTool) {
  return (await processTable(runTool)).filter(
    ({ pgid }) => pgid === session.pgid,
  );
}

export async function findOwnedDaemon(session, runTool, signal) {
  if (!DIGEST.test(session.launchId ?? "")) fail("daemon_generation_invalid");
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    const candidates = (await processTable(runTool)).filter(
      ({ ppid, command }) =>
        ppid === session.pid &&
        exactExecutableCommand(command, session.executablePaths.daemon) &&
        command.includes("--data-dir") &&
        command.includes(session.dataDir) &&
        command.includes("--launch-id") &&
        command.includes(session.launchId),
    );
    if (candidates.length === 1) {
      session.trackedDaemonPid = candidates[0].pid;
      return Object.freeze({
        pid: candidates[0].pid,
        pgid: candidates[0].pgid,
        start_time: await readProcessStartTime(candidates[0].pid, runTool),
        executable: session.executablePaths.daemon,
        session_authority: session.sessionAuthority,
      });
    }
    if (candidates.length > 1) fail("daemon_process_ambiguous");
    await wait(POLL_MS);
  }
  fail("owned_daemon_not_found");
}

export async function strongKillOwnedDaemon(session, target, runTool) {
  if (
    target?.session_authority !== session.sessionAuthority ||
    !DIGEST.test(session.launchId ?? "")
  ) {
    fail("owned_daemon_identity_changed");
  }
  const current = (await processTable(runTool)).find(
    ({ pid, ppid, pgid, command }) =>
      pid === target.pid &&
      ppid === session.pid &&
      pgid === target.pgid &&
      exactExecutableCommand(command, session.executablePaths.daemon) &&
      command.includes(session.dataDir) &&
      command.includes("--launch-id") &&
      command.includes(session.launchId),
  );
  if (!current) fail("owned_daemon_identity_changed");
  await requireRecordedProcessStart(
    target,
    runTool,
    "owned_daemon_identity_changed",
  );
  try {
    process.kill(target.pid, "SIGKILL");
  } catch {
    fail("owned_daemon_kill_failed");
  }
}

export function selectSessionApplication(processes, session) {
  const process = processes.find(({ pid }) => pid === session.pid);
  if (
    !process ||
    process.pgid !== session.pgid ||
    !exactApplicationCommand(
      process.command,
      session.executablePaths.desktop,
      session.sessionAuthority,
    )
  ) {
    fail("normal_quit_identity_changed");
  }
  return process;
}

export async function quitInstalledApp(session, runTool) {
  if (session.stopped) return;
  if (
    session.applicationRecord?.session_authority !==
      session.sessionAuthority ||
    session.applicationRecord?.executable !== session.executablePaths.desktop
  ) {
    fail("normal_quit_identity_changed");
  }
  selectSessionApplication(await processTable(runTool), session);
  await requireRecordedProcessStart(
    session.applicationRecord,
    runTool,
    "normal_quit_identity_changed",
  );
  const result = await runTool(
    "/usr/bin/osascript",
    [
      "-l",
      "JavaScript",
      QUIT_INSTALLED_APP_SCRIPT,
      String(session.pid),
      session.executablePaths.desktop,
    ],
    { timeoutMs: 5_000 },
  );
  if (
    !toolSucceeded(result) ||
    result.stdout !== TARGETED_TERMINATE_ACCEPTED ||
    result.stderr !== ""
  ) {
    fail("normal_quit_failed");
  }
  const deadline = Date.now() + QUIT_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (
      (await installedRuntimeProcesses(runTool, session.executablePaths))
        .length === 0
    ) {
      await terminateGuardian(session.child, session.monitor);
      if ((await groupProcesses(session, runTool)).length === 0) {
        session.stopped = true;
        return;
      }
    }
    await wait(POLL_MS);
  }
  fail("normal_quit_residue");
}

export async function forceCleanupSession(session, runTool) {
  if (session.stopped) return;
  try {
    const current = (await processTable(runTool)).find(
      ({ pid }) => pid === session.guardianRecord?.pid,
    );
    if (current) {
      const expectedCommand =
        `${process.execPath} ${APP_GUARDIAN_FILE} ` +
        `--session-authority ${session.sessionAuthority} ` +
        `--desktop-executable ${session.executablePaths.desktop} ` +
        `--home ${session.home}`;
      if (
        current.pgid !== session.guardianRecord.pgid ||
        current.command !== expectedCommand ||
        session.guardianRecord.session_authority !== session.sessionAuthority ||
        session.guardianRecord.executable !== process.execPath
      ) {
        fail("cleanup_failed");
      }
      await requireRecordedProcessStart(
        session.guardianRecord,
        runTool,
        "cleanup_failed",
      );
    } else {
      const anchor = (await processTable(runTool)).find(
        ({ pid }) => pid === session.anchorRecord?.pid,
      );
      if (anchor) {
        if (
          anchor.pgid !== session.anchorRecord.pgid ||
          anchor.command !== authorityAnchorCommand(session.sessionAuthority) ||
          session.anchorRecord.session_authority !== session.sessionAuthority ||
          session.anchorRecord.executable !== process.execPath
        ) {
          fail("cleanup_failed");
        }
        await requireRecordedProcessStart(
          session.anchorRecord,
          runTool,
          "cleanup_failed",
        );
      } else {
        const app = (await processTable(runTool)).find(
          ({ pid }) => pid === session.applicationRecord?.pid,
        );
        if (app) {
          if (
            app.pgid !== session.applicationRecord.pgid ||
            !exactApplicationCommand(
              app.command,
              session.applicationRecord.executable,
              session.sessionAuthority,
            ) ||
            session.applicationRecord.session_authority !==
              session.sessionAuthority
          ) {
            fail("cleanup_failed");
          }
          await requireRecordedProcessStart(
            session.applicationRecord,
            runTool,
            "cleanup_failed",
          );
        } else {
          session.stopped =
            (await installedRuntimeProcesses(runTool, session.executablePaths))
              .length === 0;
          if (session.stopped) return;
          fail("cleanup_failed");
        }
      }
    }
    signalProcessGroup(session.child, "SIGTERM");
    await waitBounded(session.monitor.promise, 2_000);
    if ((await groupProcesses(session, runTool)).length > 0) {
      signalProcessGroup(session.child, "SIGKILL");
      await waitBounded(session.monitor.promise, 1_000);
    }
    session.stopped =
      (await installedRuntimeProcesses(runTool, session.executablePaths))
        .length === 0 && (await groupProcesses(session, runTool)).length === 0;
  } catch {
    session.stopped = false;
  }
  if (!session.stopped) fail("cleanup_failed");
}

export async function recordWorkspaceApplication(
  workspace,
  session,
  state = "app_running",
) {
  if (state === "app_running") {
    if (
      !session.guardianRecord ||
      !session.anchorRecord ||
      !session.applicationRecord
    ) {
      fail("workspace_marker_invalid");
    }
    await updateWorkspaceMarker(
      workspace.root,
      workspace.runId,
      {
        state,
        session_authority: session.sessionAuthority,
        guardian: session.guardianRecord,
        authority_anchor: session.anchorRecord,
        application: session.applicationRecord,
      },
    );
    return;
  }
  if (state !== "app_stopped") fail("workspace_marker_invalid");
  await updateWorkspaceMarker(
    workspace.root,
    workspace.runId,
    {
      state,
      session_authority: null,
      guardian: null,
      authority_anchor: null,
      application: null,
    },
  );
}

function exactCloneHelper(command, workspace, entryScriptFile) {
  const prefix = `${process.execPath} ${entryScriptFile} --internal-cow-clone `;
  return command.startsWith(prefix) && command.endsWith(` ${workspace.dataDir}`);
}

function exactGuardian(command, workspace, marker, executablePaths) {
  return command ===
    `${marker.guardian?.executable ?? process.execPath} ${APP_GUARDIAN_FILE} ` +
      `--session-authority ${marker.session_authority} ` +
      `--desktop-executable ${executablePaths.desktop} --home ${workspace.home}`;
}

function exactAuthorityAnchor(command, marker) {
  return command === authorityAnchorCommand(marker.session_authority);
}

function exactLockHelper(command, workspace, marker, flockHolderFile) {
  const parts =
    marker.helper.lock_kind === "fulltext"
      ? ["search-index", "snapshot-publication.lock"]
      : ["vector-index", "snapshot-publication.lock"];
  const lockFile = path.join(workspace.dataDir, ...parts);
  return command === `/usr/bin/ruby ${flockHolderFile} ${lockFile}`;
}

async function terminateExactRecordedGroup(record, runTool, killProcess) {
  const recordedAlive = async () => {
    const current = (await processTable(runTool)).find(
      ({ pid }) => pid === record.pid,
    );
    if (!current) return false;
    if (
      current.pgid !== record.pgid ||
      !exactExecutableCommand(current.command, record.executable)
    ) {
      fail("stale_workspace_recovery_failed");
    }
    await requireRecordedProcessStart(
      record,
      runTool,
      "stale_workspace_recovery_failed",
    );
    return true;
  };
  if (!(await recordedAlive())) return;
  const send = (signal) => {
    try {
      killProcess(-record.pgid, signal);
    } catch {
      fail("stale_workspace_recovery_failed");
    }
  };
  send("SIGTERM");
  for (let attempt = 0; attempt < 10; attempt += 1) {
    if (!(await recordedAlive())) return;
    await wait(POLL_MS);
  }
  if (!(await recordedAlive())) return;
  send("SIGKILL");
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (!(await recordedAlive())) return;
    await wait(POLL_MS);
  }
  fail("stale_workspace_recovery_failed");
}

async function terminateVerifiedAuthorityGroup(
  record,
  commandMatches,
  runTool,
  killProcess,
) {
  const authority = (await processTable(runTool)).find(
    ({ pid }) => pid === record.pid,
  );
  if (
    !authority ||
    authority.pgid !== record.pgid ||
    !commandMatches(authority.command)
  ) {
    fail("stale_workspace_recovery_failed");
  }
  await requireRecordedProcessStart(
    record,
    runTool,
    "stale_workspace_recovery_failed",
  );
  const remaining = async () =>
    (await processTable(runTool)).filter(({ pgid }) => pgid === record.pgid);
  const send = async (signal) => {
    try {
      killProcess(-record.pgid, signal);
    } catch {
      if ((await remaining()).length === 0) return;
      fail("stale_workspace_recovery_failed");
    }
  };
  await send("SIGTERM");
  for (let attempt = 0; attempt < 10; attempt += 1) {
    if ((await remaining()).length === 0) return;
    await wait(POLL_MS);
  }
  await send("SIGKILL");
  for (let attempt = 0; attempt < 10; attempt += 1) {
    if ((await remaining()).length === 0) return;
    await wait(POLL_MS);
  }
  fail("stale_workspace_recovery_failed");
}

export async function recoverStaleWorkspaceRuntime(
  workspace,
  runTool,
  {
    entryScriptFile = ENTRY_SCRIPT_FILE,
    executablePaths = DEFAULT_INSTALLED_EXECUTABLES,
    flockHolderFile = FLOCK_HOLDER_FILE,
    killProcess = process.kill.bind(process),
  } = {},
) {
  const marker = validateWorkspaceMarker(workspace?.marker);
  const processes = await processTable(runTool);
  let guardianRecord = marker.guardian;
  let guardian = guardianRecord
    ? processes.find(({ pid }) => pid === guardianRecord.pid)
    : undefined;
  if (
    guardian &&
    (guardian.pgid !== marker.guardian.pgid ||
      !exactGuardian(guardian.command, workspace, marker, executablePaths))
  ) {
    fail("stale_workspace_recovery_failed");
  }
  if (guardian) {
    await requireRecordedProcessStart(
      marker.guardian,
      runTool,
      "stale_workspace_recovery_failed",
    );
  }
  if (marker.state === "launch_intent") {
    const candidates = processes.filter((process) =>
      exactGuardian(process.command, workspace, marker, executablePaths),
    );
    if (candidates.length > 1) fail("stale_workspace_recovery_failed");
    if (candidates.length === 1) {
      guardian = candidates[0];
      guardianRecord = {
        pid: guardian.pid,
        pgid: guardian.pgid,
        start_time: await readProcessStartTime(guardian.pid, runTool),
        executable: process.execPath,
        session_authority: marker.session_authority,
      };
    }
  }
  let anchorRecord = marker.authority_anchor;
  let anchor = anchorRecord
    ? processes.find(({ pid }) => pid === anchorRecord.pid)
    : undefined;
  if (
    anchor &&
    (anchor.pgid !== marker.guardian?.pgid ||
      !exactAuthorityAnchor(anchor.command, marker))
  ) {
    fail("stale_workspace_recovery_failed");
  }
  if (anchor) {
    await requireRecordedProcessStart(
      anchorRecord,
      runTool,
      "stale_workspace_recovery_failed",
    );
  } else if (marker.guardian) {
    const candidates = processes.filter(
      ({ pgid, command }) =>
        pgid === marker.guardian.pgid &&
        exactAuthorityAnchor(command, marker),
    );
    if (candidates.length > 1) fail("stale_workspace_recovery_failed");
    if (candidates.length === 1) {
      anchor = candidates[0];
      anchorRecord = {
        pid: anchor.pid,
        pgid: anchor.pgid,
        start_time: await readProcessStartTime(anchor.pid, runTool),
        executable: process.execPath,
        session_authority: marker.session_authority,
      };
    }
  }
  let app = marker.application
    ? processes.find(({ pid }) => pid === marker.application.pid)
    : undefined;
  if (
    app &&
    (app.pgid !== marker.application.pgid ||
      !exactApplicationCommand(
        app.command,
        executablePaths.desktop,
        marker.session_authority,
      ))
  ) {
    fail("stale_workspace_recovery_failed");
  }
  if (app) {
    await requireRecordedProcessStart(
      marker.application,
      runTool,
      "stale_workspace_recovery_failed",
    );
  }
  const recordedAuthorityPgid = marker.guardian?.pgid;
  if (
    !guardian &&
    !anchor &&
    !app &&
    Number.isSafeInteger(recordedAuthorityPgid) &&
    processes.some(({ pgid }) => pgid === recordedAuthorityPgid)
  ) {
    fail("stale_workspace_recovery_failed");
  }
  if (!app && marker.state === "launch_pending" && marker.guardian) {
    const candidates = processes.filter(
      ({ pgid, command }) =>
        pgid === marker.guardian.pgid &&
        exactApplicationCommand(
          command,
          executablePaths.desktop,
          marker.session_authority,
        ),
    );
    if (candidates.length > 1) fail("stale_workspace_recovery_failed");
    if (candidates.length === 1) {
      app = candidates[0];
    }
  }
  if (guardian || anchor || app) {
    const appPid = app?.pid ?? marker.application?.pid;
    const daemons = processes.filter(
      ({ ppid, command }) =>
        ppid === appPid &&
        exactExecutableCommand(command, executablePaths.daemon) &&
        command.includes(`--data-dir ${workspace.dataDir}`),
    );
    if (daemons.length > 1) fail("stale_workspace_recovery_failed");
    const sidecars = processes.filter(({ ppid, command }) =>
      daemons.some(({ pid }) => pid === ppid) &&
      (exactExecutableCommand(
        command,
        executablePaths.embedding_runtime,
      ) ||
        exactExecutableCommand(
          command,
          executablePaths.pdf_renderer,
        )),
    );
    const allowed = new Set(
      [guardian, anchor, app, ...daemons, ...sidecars]
        .filter(Boolean)
        .map(({ pid }) => pid),
    );
    const authorityPgid = guardianRecord?.pgid ?? guardian?.pgid;
    const installedExecutables = executableList(executablePaths);
    for (const process of processes) {
      if (process.pgid !== authorityPgid || allowed.has(process.pid)) continue;
      if (
        !installedExecutables.some((executable) =>
          exactExecutableCommand(process.command, executable),
        )
      ) {
        fail("stale_workspace_recovery_failed");
      }
      allowed.add(process.pid);
    }
    if (
      processes.some(
        ({ pid, pgid }) => pgid === authorityPgid && !allowed.has(pid),
      )
    ) {
      fail("stale_workspace_recovery_failed");
    }
    const primaryRecord = guardian
      ? guardianRecord
      : anchor
        ? anchorRecord
        : marker.application ?? {
            pid: app.pid,
            pgid: app.pgid,
            start_time: await readProcessStartTime(app.pid, runTool),
            executable: executablePaths.desktop,
            session_authority: marker.session_authority,
          };
    const primaryCommandMatches = guardian
      ? (command) => exactGuardian(command, workspace, marker, executablePaths)
      : anchor
        ? (command) => exactAuthorityAnchor(command, marker)
        : (command) =>
            exactApplicationCommand(
              command,
              executablePaths.desktop,
              marker.session_authority,
            );
    const daemonRecords = await Promise.all(
      daemons.map(async (daemon) => ({
        pid: daemon.pid,
        pgid: daemon.pgid,
        start_time: await readProcessStartTime(daemon.pid, runTool),
        executable: executablePaths.daemon,
        session_authority: marker.session_authority,
      })),
    );
    await terminateVerifiedAuthorityGroup(
      primaryRecord,
      primaryCommandMatches,
      runTool,
      killProcess,
    );
    for (const daemonRecord of daemonRecords) {
      await terminateExactRecordedGroup(daemonRecord, runTool, killProcess);
    }
  }
  if (marker.helper !== null) {
    const helperProcesses = await processTable(runTool);
    const helper = helperProcesses.find(({ pid }) => pid === marker.helper.pid);
    if (
      helper &&
      (helper.pgid !== marker.helper.pgid ||
        !exactExecutableCommand(helper.command, marker.helper.executable) ||
        !(marker.helper.kind === "cow_clone"
          ? exactCloneHelper(helper.command, workspace, entryScriptFile)
          : exactLockHelper(
              helper.command,
              workspace,
              marker,
              flockHolderFile,
            )))
    ) {
      fail("stale_workspace_recovery_failed");
    }
    if (helper) {
      await requireRecordedProcessStart(
        marker.helper,
        runTool,
        "stale_workspace_recovery_failed",
      );
      await terminateExactRecordedGroup(marker.helper, runTool, killProcess);
    }
  }
}
