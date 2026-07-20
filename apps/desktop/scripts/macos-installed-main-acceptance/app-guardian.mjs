#!/usr/bin/env node

import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  APP_DATA_DIRECTORY,
  MAX_TOOL_OUTPUT_BYTES,
  RUN_ID,
  exactKeys,
  fail,
  validAbsolutePath,
} from "./core.mjs";

export const APP_GUARDIAN_FILE = fileURLToPath(import.meta.url);
const HANDSHAKE_SCHEMA = "resume-ir.app-guardian.v1";
const COMMIT_PREFIX = "resume-ir.app-guardian.commit.v1";
const AUTHORITY_ANCHOR_ARGUMENT = "--authority-anchor";

export function authorityAnchorCommand(authority) {
  if (!RUN_ID.test(authority ?? "")) fail("app_guardian_protocol_invalid");
  return `${process.execPath} ${APP_GUARDIAN_FILE} ${AUTHORITY_ANCHOR_ARGUMENT} ${authority}`;
}

export function guardianCommitLine(authority) {
  if (!RUN_ID.test(authority ?? "")) fail("app_guardian_protocol_invalid");
  return `${COMMIT_PREFIX} ${authority}\n`;
}

export function parseGuardianHandshake(source, expectedAuthority) {
  if (
    typeof source !== "string" ||
    Buffer.byteLength(source, "utf8") > 1_024 ||
    !source.endsWith("\n") ||
    source.slice(0, -1).includes("\n") ||
    !RUN_ID.test(expectedAuthority ?? "")
  ) {
    fail("app_guardian_protocol_invalid");
  }
  let value;
  try {
    value = JSON.parse(source.slice(0, -1));
  } catch {
    fail("app_guardian_protocol_invalid");
  }
  if (
    !exactKeys(value, [
      "schema_version",
      "session_authority",
      "anchor_pid",
      "app_pid",
    ]) ||
    value.schema_version !== HANDSHAKE_SCHEMA ||
    value.session_authority !== expectedAuthority ||
    !Number.isSafeInteger(value.anchor_pid) ||
    value.anchor_pid <= 1 ||
    !Number.isSafeInteger(value.app_pid) ||
    value.app_pid <= 1
  ) {
    fail("app_guardian_protocol_invalid");
  }
  return Object.freeze({ anchorPid: value.anchor_pid, appPid: value.app_pid });
}

function parseArguments(argv) {
  if (
    argv.length !== 6 ||
    argv[0] !== "--session-authority" ||
    !RUN_ID.test(argv[1] ?? "") ||
    argv[2] !== "--desktop-executable" ||
    !validAbsolutePath(argv[3]) ||
    argv[4] !== "--home" ||
    !validAbsolutePath(argv[5])
  ) {
    return undefined;
  }
  return Object.freeze({
    authority: argv[1],
    desktopExecutable: argv[3],
    home: argv[5],
  });
}

function appEnvironment(home, authority) {
  return {
    HOME: home,
    LANG: "C",
    LC_ALL: "C",
    PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
    TMPDIR: "/tmp",
    RESUME_IR_ACCEPTANCE_SESSION_AUTHORITY: authority,
  };
}

async function runGuardian(argv) {
  const options = parseArguments(argv);
  if (!options) return 64;
  const expectedDataDir = path.join(
    options.home,
    "Library",
    "Application Support",
    APP_DATA_DIRECTORY,
  );
  if (!validAbsolutePath(expectedDataDir)) return 64;
  let app;
  let anchor;
  let committed = false;
  let finishing = false;
  let input = "";

  const waitForExit = (timeoutMs) =>
    new Promise((resolve) => {
      if (!app || app.exitCode !== null || app.signalCode !== null) {
        resolve(true);
        return;
      }
      const timer = setTimeout(() => resolve(false), timeoutMs);
      app.once("exit", () => {
        clearTimeout(timer);
        resolve(true);
      });
    });
  const finish = async () => {
    if (finishing) return;
    finishing = true;
    try {
      process.kill(-process.pid, "SIGTERM");
    } catch {
      process.exit(70);
    }
    await waitForExit(1_000);
    try {
      process.kill(-process.pid, "SIGKILL");
    } catch {
      process.exit(70);
    }
  };
  process.on("SIGINT", () => void finish());
  process.on("SIGTERM", () => void finish());
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => {
    if (finishing || committed) {
      if (Buffer.byteLength(chunk, "utf8") > 1_024) void finish();
      return;
    }
    input += chunk;
    if (Buffer.byteLength(input, "utf8") > 1_024) {
      void finish();
      return;
    }
    if (!input.includes("\n")) return;
    if (input !== guardianCommitLine(options.authority)) {
      void finish();
      return;
    }
    committed = true;
    try {
      anchor = spawn(
        process.execPath,
        [APP_GUARDIAN_FILE, AUTHORITY_ANCHOR_ARGUMENT, options.authority],
        {
          cwd: "/",
          detached: false,
          env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin" },
          shell: false,
          stdio: "ignore",
          windowsHide: true,
        },
      );
      app = spawn(
        options.desktopExecutable,
        [`--resume-ir-acceptance-session-authority=${options.authority}`],
        {
          cwd: "/",
          detached: false,
          env: appEnvironment(options.home, options.authority),
          shell: false,
          stdio: ["ignore", "ignore", "pipe"],
          windowsHide: true,
        },
      );
    } catch {
      void finish();
      return;
    }
    let stderrBytes = 0;
    app.stderr?.on("data", (stderrChunk) => {
      stderrBytes += stderrChunk.length;
      if (stderrBytes > MAX_TOOL_OUTPUT_BYTES) void finish();
    });
    app.stderr?.on("error", () => void finish());
    anchor.once("error", () => void finish());
    anchor.once("exit", () => {
      if (!finishing) void finish();
    });
    app.once("error", () => void finish());
    app.once("exit", () => {
      if (!finishing) void finish();
    });
    process.stdout.write(
      `${JSON.stringify({
        schema_version: HANDSHAKE_SCHEMA,
        session_authority: options.authority,
        anchor_pid: anchor.pid,
        app_pid: app.pid,
      })}\n`,
    );
  });
  process.stdin.once("end", () => void finish());
  process.stdin.once("error", () => void finish());
  process.stdin.resume();
  await new Promise(() => {});
}

async function runAuthorityAnchor(argv) {
  if (argv.length !== 1 || !RUN_ID.test(argv[0] ?? "")) return 64;
  await new Promise(() => setInterval(() => {}, 60_000));
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const argv = process.argv.slice(2);
  process.exitCode =
    argv[0] === AUTHORITY_ANCHOR_ARGUMENT
      ? await runAuthorityAnchor(argv.slice(1))
      : await runGuardian(argv);
}
