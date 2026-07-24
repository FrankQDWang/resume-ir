import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import { lstat, mkdir, open } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { runBoundedTool, toolSucceeded } from "./bounded-process.mjs";
import {
  LIFECYCLE_RECEIPT,
  AcceptanceError,
  exactKeys,
  fail,
  throwIfAborted,
  wait,
} from "./core.mjs";
import { SYNTHETIC_CANARY_TOKEN } from "./acceptance-evidence.mjs";
import { readPrivateJson } from "./filesystem-cow.mjs";
import {
  readDaemonConnection,
  validateDaemonDiagnostics,
} from "./ipc-contracts.mjs";

const EXPORT_SCRIPT = fileURLToPath(
  new URL("./export-diagnostics.jxa", import.meta.url),
);
const EXPORT_COMPLETE = "resume-ir.native-diagnostics-export.complete.v1\n";
const DESKTOP_DIAGNOSTICS_SCHEMA = "resume-ir.desktop-diagnostics.v2";
const LIFECYCLE_SCHEMA = "resume-ir.desktop-daemon-lifecycle-receipt.v2";
const MAX_EXPORT_BYTES = 256 * 1024;
const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;
const NATIVE_EXPORT_TIMEOUT_MS = 30_000;
const CIRCUIT_OBSERVATION_TIMEOUT_MS = 10_000;
const RESTART_FAILURES_TO_OPEN_CIRCUIT = 6;

const REASONS_BY_STATE = Object.freeze({
  starting: ["initial_start", "automatic_retry", "manual_retry", "half_open_retry"],
  running: ["control_plane_ready"],
  retry_wait: [
    "child_exited",
    "startup_timeout",
    "heartbeat_timeout",
    "start_failed",
    "control_plane_failure",
  ],
  circuit_open: ["restart_budget_exhausted"],
  blocked: [
    "configuration_invalid",
    "runtime_integrity",
    "protocol_mismatch",
    "ownership_conflict",
    "supervisor_unavailable",
  ],
});
const EXIT_REASONS = Object.freeze([
  "child_exited",
  "startup_timeout",
  "heartbeat_timeout",
  "start_failed",
  "control_plane_failure",
]);

function boundedInteger(value, maximum = MAX_SAFE_INTEGER) {
  return Number.isSafeInteger(value) && value >= 0 && value <= maximum;
}

function lifecycleEventValid(event) {
  return (
    exactKeys(event, [
      "at_unix_ms",
      "state",
      "transition_reason",
      "generation",
      "automatic_restart_attempt",
      "automatic_restart_limit",
      "retry_after_ms",
      "heartbeat_failures",
      "last_exit",
    ]) &&
    boundedInteger(event.at_unix_ms) &&
    boundedInteger(event.generation) &&
    REASONS_BY_STATE[event.state]?.includes(event.transition_reason) === true &&
    boundedInteger(event.automatic_restart_attempt, 5) &&
    event.automatic_restart_limit === 5 &&
    (event.retry_after_ms === null || boundedInteger(event.retry_after_ms, 300_000)) &&
    boundedInteger(event.heartbeat_failures, 255) &&
    (event.last_exit === null || EXIT_REASONS.includes(event.last_exit)) &&
    (["retry_wait", "circuit_open"].includes(event.state)
      ? event.retry_after_ms !== null
      : event.retry_after_ms === null)
  );
}

function lifecycleAggregateValid(value) {
  return (
    exactKeys(value, [
      "schema_version",
      "persistence_state",
      "dropped_event_count",
      "retained_event_count",
      "events",
    ]) &&
    value.schema_version === LIFECYCLE_SCHEMA &&
    ["ready", "recovered_corrupt", "unavailable"].includes(
      value.persistence_state,
    ) &&
    boundedInteger(value.dropped_event_count) &&
    boundedInteger(value.retained_event_count, 16) &&
    Array.isArray(value.events) &&
    value.events.length === value.retained_event_count &&
    value.events.every(lifecycleEventValid)
  );
}

function scanForPrivateStrings(value, forbidden) {
  const visit = (candidate) => {
    if (typeof candidate === "string") {
      if (
        candidate.startsWith("/") ||
        /^[A-Za-z]:[\\/]/.test(candidate) ||
        candidate.startsWith("file:") ||
        forbidden.some(
          (secret) =>
            typeof secret === "string" &&
            secret.length > 0 &&
            candidate.includes(secret),
        )
      ) {
        fail("desktop_diagnostics_privacy_invalid");
      }
      return;
    }
    if (Array.isArray(candidate)) {
      candidate.forEach(visit);
      return;
    }
    if (candidate !== null && typeof candidate === "object") {
      Object.values(candidate).forEach(visit);
    }
  };
  visit(value);
}

export function validateCombinedDiagnosticsDocument(
  value,
  { expectedDaemonState, forbidden = [] },
) {
  const daemonDiagnostics = value?.daemon_diagnostics;
  const serialized = JSON.stringify(value);
  if (
    !["included", "unavailable"].includes(expectedDaemonState) ||
    typeof serialized !== "string" ||
    Buffer.byteLength(serialized, "utf8") > MAX_EXPORT_BYTES ||
    !exactKeys(value, [
      "schema_version",
      "privacy_boundary",
      "contains_raw_resume_text",
      "contains_queries",
      "contains_resume_paths",
      "contains_candidate_results",
      "contains_snippet_text",
      "lifecycle",
      "daemon_diagnostics_state",
      "daemon_diagnostics",
    ]) ||
    value.schema_version !== DESKTOP_DIAGNOSTICS_SCHEMA ||
    value.privacy_boundary !== "redacted_local_aggregate" ||
    [
      value.contains_raw_resume_text,
      value.contains_queries,
      value.contains_resume_paths,
      value.contains_candidate_results,
      value.contains_snippet_text,
    ].some((flag) => flag !== false) ||
    !lifecycleAggregateValid(value.lifecycle) ||
    value.daemon_diagnostics_state !== expectedDaemonState ||
    (expectedDaemonState === "included" && daemonDiagnostics === null) ||
    (expectedDaemonState === "unavailable" && daemonDiagnostics !== null)
  ) {
    fail("desktop_diagnostics_contract_invalid");
  }
  if (expectedDaemonState === "included") {
    validateDaemonDiagnostics(daemonDiagnostics, forbidden);
  }
  scanForPrivateStrings(value, forbidden);
  return value;
}

async function requirePrivateExportDirectory(directory) {
  await mkdir(directory, { mode: 0o700 });
  const metadata = await lstat(directory);
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    (metadata.mode & 0o077) !== 0
  ) {
    fail("desktop_diagnostics_export_directory_invalid");
  }
}

async function readVerifiedExport(file, expectedDaemonState, forbidden) {
  let handle;
  try {
    handle = await open(
      file,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
    );
    const before = await handle.stat();
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.size < 2 ||
      before.size > MAX_EXPORT_BYTES ||
      (before.mode & 0o077) !== 0
    ) {
      fail("desktop_diagnostics_export_file_invalid");
    }
    const body = await handle.readFile();
    const after = await handle.stat();
    if (
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.size !== after.size ||
      body.length !== before.size
    ) {
      fail("desktop_diagnostics_export_file_changed");
    }
    let value;
    try {
      value = JSON.parse(body.toString("utf8"));
    } catch {
      fail("desktop_diagnostics_export_json_invalid");
    }
    return validateCombinedDiagnosticsDocument(value, {
      expectedDaemonState,
      forbidden,
    });
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail("desktop_diagnostics_export_file_invalid");
  } finally {
    await handle?.close().catch(() => {});
  }
}

async function nativeExport(
  { session, expectedDaemonState, exportDirectory, fileName, signal },
  runTool,
) {
  const result = await runTool(
    "/usr/bin/osascript",
    [
      "-l",
      "JavaScript",
      EXPORT_SCRIPT,
      String(session.pid),
      session.executablePaths.desktop,
      expectedDaemonState,
      exportDirectory,
      fileName,
    ],
    {
      env: { LANG: "C", LC_ALL: "C", PATH: "/usr/bin:/bin" },
      signal,
      timeoutMs: NATIVE_EXPORT_TIMEOUT_MS,
    },
  );
  if (
    !toolSucceeded(result) ||
    result.stdout !== EXPORT_COMPLETE ||
    result.stderr !== ""
  ) {
    fail("desktop_diagnostics_native_export_failed");
  }
}

async function waitForCircuitOpen(session, signal) {
  const deadline = Date.now() + CIRCUIT_OBSERVATION_TIMEOUT_MS;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    try {
      const receipt = await readPrivateJson(
        path.join(session.dataDir, LIFECYCLE_RECEIPT),
        16 * 1024,
      );
      const events = receipt.value?.events;
      const last = Array.isArray(events) ? events.at(-1) : null;
      if (
        receipt.value?.schema_version === LIFECYCLE_SCHEMA &&
        last?.state === "circuit_open" &&
        last?.transition_reason === "restart_budget_exhausted" &&
        last?.automatic_restart_attempt === 5
      ) {
        return;
      }
    } catch (error) {
      if (
        error instanceof AcceptanceError &&
        !["private_file_invalid", "private_json_invalid"].includes(error.code)
      ) {
        throw error;
      }
    }
    await wait(100);
  }
  fail("desktop_diagnostics_daemon_down_not_observed");
}

async function exhaustRestartBudget(runtime, session, signal) {
  const generationSecrets = [];
  for (
    let failure = 0;
    failure < RESTART_FAILURES_TO_OPEN_CIRCUIT;
    failure += 1
  ) {
    throwIfAborted(signal);
    const oldInstanceId = session.instanceId;
    const target = await runtime.findOwnedDaemon(session);
    await runtime.strongKillDaemon(session, target);
    if (failure + 1 < RESTART_FAILURES_TO_OPEN_CIRCUIT) {
      const recovered = await runtime.waitForNewGenerationReady(
        session,
        oldInstanceId,
      );
      generationSecrets.push(
        recovered.connection?.token,
        recovered.instanceId,
        recovered.launchId,
      );
    }
  }
  return generationSecrets;
}

function verifiedEvidence() {
  return Object.freeze({
    desktopContract: DESKTOP_DIAGNOSTICS_SCHEMA,
    nativeSaveDialog: true,
    ownerOnlyFile: true,
    boundedBytes: MAX_EXPORT_BYTES,
    daemonAvailableState: "included",
    daemonUnavailableState: "unavailable",
    daemonDownLifecycleState: "circuit_open",
  });
}

export async function verifyInstalledCombinedDiagnosticsExport(
  runtime,
  session,
  signal,
  dependencies = {},
) {
  if (typeof runtime.verifyCombinedDiagnosticsExport === "function") {
    return runtime.verifyCombinedDiagnosticsExport(session);
  }
  if (
    !Number.isSafeInteger(session?.pid) ||
    session.pid <= 1 ||
    typeof session?.workspace?.root !== "string" ||
    typeof session?.dataDir !== "string" ||
    typeof session?.home !== "string" ||
    typeof session?.executablePaths?.desktop !== "string"
  ) {
    fail("desktop_diagnostics_session_invalid");
  }
  const runTool = dependencies.runTool ?? runBoundedTool;
  const exportThroughUi = dependencies.nativeExport ?? nativeExport;
  const observeCircuit = dependencies.waitForCircuitOpen ?? waitForCircuitOpen;
  const readConnection = dependencies.readDaemonConnection ?? readDaemonConnection;
  const exportDirectory = path.join(
    session.workspace.root,
    "diagnostics-acceptance",
  );
  await requirePrivateExportDirectory(exportDirectory);

  const readyConnection = await readConnection(session.dataDir);
  const forbidden = [
    session.workspace.root,
    session.dataDir,
    session.home,
    SYNTHETIC_CANARY_TOKEN,
    readyConnection.token,
    readyConnection.instanceId,
    readyConnection.launchId,
  ];
  const includedName = `daemon-included-${randomBytes(16).toString("hex")}.json`;
  await exportThroughUi(
    {
      session,
      expectedDaemonState: "included",
      exportDirectory,
      fileName: includedName,
      signal,
    },
    runTool,
  );
  await readVerifiedExport(
    path.join(exportDirectory, includedName),
    "included",
    forbidden,
  );

  forbidden.push(...(await exhaustRestartBudget(runtime, session, signal)));
  await observeCircuit(session, signal);
  const unavailableName =
    `daemon-unavailable-${randomBytes(16).toString("hex")}.json`;
  await exportThroughUi(
    {
      session,
      expectedDaemonState: "unavailable",
      exportDirectory,
      fileName: unavailableName,
      signal,
    },
    runTool,
  );
  const unavailable = await readVerifiedExport(
    path.join(exportDirectory, unavailableName),
    "unavailable",
    forbidden,
  );
  if (
    !unavailable.lifecycle.events.some(
      (event) =>
        event.state === "circuit_open" &&
        event.transition_reason === "restart_budget_exhausted" &&
        event.automatic_restart_attempt === 5,
    )
  ) {
    fail("desktop_diagnostics_daemon_down_receipt_missing");
  }
  return verifiedEvidence();
}
