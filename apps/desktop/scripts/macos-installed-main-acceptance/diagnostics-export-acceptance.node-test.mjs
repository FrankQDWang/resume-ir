import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  validateCombinedDiagnosticsDocument,
  verifyInstalledCombinedDiagnosticsExport,
} from "./diagnostics-export-acceptance.mjs";
import { diagnostics } from "./fixtures.mjs";

function lifecycleEvent(overrides = {}) {
  return {
    at_unix_ms: 1_700_000_000_000,
    state: "running",
    transition_reason: "control_plane_ready",
    generation: 1,
    automatic_restart_attempt: 0,
    automatic_restart_limit: 5,
    retry_after_ms: null,
    heartbeat_failures: 0,
    last_exit: null,
    ...overrides,
  };
}

function combinedDiagnostics(daemonState, overrides = {}) {
  const events =
    daemonState === "unavailable"
      ? [
          lifecycleEvent({
            state: "circuit_open",
            transition_reason: "restart_budget_exhausted",
            automatic_restart_attempt: 5,
            retry_after_ms: 300_000,
            last_exit: "child_exited",
          }),
        ]
      : [lifecycleEvent()];
  return {
    schema_version: "resume-ir.desktop-diagnostics.v2",
    privacy_boundary: "redacted_local_aggregate",
    contains_raw_resume_text: false,
    contains_queries: false,
    contains_resume_paths: false,
    contains_candidate_results: false,
    contains_snippet_text: false,
    lifecycle: {
      schema_version: "resume-ir.desktop-daemon-lifecycle-receipt.v2",
      persistence_state: "ready",
      dropped_event_count: 0,
      retained_event_count: events.length,
      events,
    },
    daemon_diagnostics_state: daemonState,
    daemon_diagnostics:
      daemonState === "included" ? diagnostics() : null,
    ...overrides,
  };
}

test("combined diagnostics v2 accepts only exact included and unavailable shapes", () => {
  for (const state of ["included", "unavailable"]) {
    assert.equal(
      validateCombinedDiagnosticsDocument(combinedDiagnostics(state), {
        expectedDaemonState: state,
      }).daemon_diagnostics_state,
      state,
    );
  }

  assert.throws(
    () =>
      validateCombinedDiagnosticsDocument(
        combinedDiagnostics("included", { unknown: true }),
        { expectedDaemonState: "included" },
      ),
    /desktop_diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateCombinedDiagnosticsDocument(
        combinedDiagnostics("included", {
          schema_version: "resume-ir.desktop-diagnostics.v1",
        }),
        { expectedDaemonState: "included" },
      ),
    /desktop_diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateCombinedDiagnosticsDocument(combinedDiagnostics("included"), {
        expectedDaemonState: "unavailable",
      }),
    /desktop_diagnostics_contract_invalid/,
  );
});

test("combined diagnostics rejects illegal lifecycle matrices and private strings", () => {
  const invalidLifecycle = combinedDiagnostics("unavailable");
  invalidLifecycle.lifecycle.events[0].transition_reason = "control_plane_ready";
  assert.throws(
    () =>
      validateCombinedDiagnosticsDocument(invalidLifecycle, {
        expectedDaemonState: "unavailable",
      }),
    /desktop_diagnostics_contract_invalid/,
  );

  assert.throws(
    () =>
      validateCombinedDiagnosticsDocument(combinedDiagnostics("included"), {
        expectedDaemonState: "included",
        forbidden: ["resume-ir.desktop-diagnostics"],
      }),
    /desktop_diagnostics_privacy_invalid/,
  );
});

test("native automation pins the exact PID and drives the installed save dialog", async () => {
  const source = await readFile(
    new URL("./export-diagnostics.jxa", import.meta.url),
    "utf8",
  );
  assert.match(source, /runningApplicationWithProcessIdentifier\(pid\)/);
  assert.match(source, /applicationProcesses\(\)/);
  assert.match(source, /"导出脱敏 JSON"/);
  assert.match(source, /"导出桌面生命周期诊断"/);
  assert.match(source, /"daemon 已换代，请显式重试当前操作"/);
  assert.match(source, /\["Save", "保存"\]/);
  assert.match(source, /`已导出 \$\{fileName\}`/);
  assert.doesNotMatch(source, /runningApplicationsWithBundleIdentifier/);
});

test("native acceptance exports both states and opens the sixth-failure circuit", async (context) => {
  const workspaceRoot = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-diagnostics-acceptance-"),
  );
  context.after(() => rm(workspaceRoot, { recursive: true, force: true }));
  await chmod(workspaceRoot, 0o700);
  const session = {
    pid: 42,
    workspace: { root: workspaceRoot },
    dataDir: path.join(workspaceRoot, "data"),
    home: workspaceRoot,
    executablePaths: { desktop: "/Applications/resume-ir.app/desktop" },
    instanceId: "instance-0",
    launchId: "launch-0",
  };
  let generation = 0;
  let kills = 0;
  const connection = () => ({
    token: `token-${generation}`,
    instanceId: `instance-${generation}`,
    launchId: `launch-${generation}`,
  });
  const runtime = {
    async findOwnedDaemon() {
      return { pid: 1_000 + generation };
    },
    async strongKillDaemon(_session, target) {
      assert.equal(target.pid, 1_000 + generation);
      kills += 1;
    },
    async waitForNewGenerationReady() {
      generation += 1;
      const next = connection();
      session.instanceId = next.instanceId;
      session.launchId = next.launchId;
      return { ...next, connection: next };
    },
  };
  const exportedStates = [];
  const evidence = await verifyInstalledCombinedDiagnosticsExport(
    runtime,
    session,
    undefined,
    {
      async readDaemonConnection() {
        return connection();
      },
      async runTool(command, args, toolOptions) {
        assert.equal(command, "/usr/bin/osascript");
        assert.deepEqual(args.slice(0, 2), ["-l", "JavaScript"]);
        assert.equal(args[3], "42");
        assert.equal(
          args[4],
          "/Applications/resume-ir.app/desktop",
        );
        assert.equal(toolOptions.timeoutMs, 30_000);
        const expectedDaemonState = args[5];
        const exportDirectory = args[6];
        const fileName = args[7];
        exportedStates.push(expectedDaemonState);
        const payload = combinedDiagnostics(expectedDaemonState);
        await writeFile(
          path.join(exportDirectory, fileName),
          `${JSON.stringify(payload)}\n`,
          { mode: 0o600 },
        );
        return {
          status: 0,
          stdout: "resume-ir.native-diagnostics-export.complete.v1\n",
          stderr: "",
          timedOut: false,
          overflow: false,
        };
      },
      async waitForCircuitOpen() {
        assert.equal(kills, 6);
      },
    },
  );

  assert.deepEqual(exportedStates, ["included", "unavailable"]);
  assert.equal(kills, 6);
  assert.deepEqual(evidence, {
    desktopContract: "resume-ir.desktop-diagnostics.v2",
    nativeSaveDialog: true,
    ownerOnlyFile: true,
    boundedBytes: 256 * 1024,
    daemonAvailableState: "included",
    daemonUnavailableState: "unavailable",
    daemonDownLifecycleState: "circuit_open",
  });
});

test("native acceptance rejects a group-readable export", async (context) => {
  const workspaceRoot = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-diagnostics-permissions-"),
  );
  context.after(() => rm(workspaceRoot, { recursive: true, force: true }));
  await chmod(workspaceRoot, 0o700);
  const runtime = {
    async findOwnedDaemon() {
      throw new Error("restart path must not be reached");
    },
    async strongKillDaemon() {},
    async waitForNewGenerationReady() {},
  };
  await assert.rejects(
    verifyInstalledCombinedDiagnosticsExport(
      runtime,
      {
        pid: 42,
        workspace: { root: workspaceRoot },
        dataDir: path.join(workspaceRoot, "data"),
        home: workspaceRoot,
        executablePaths: { desktop: "/Applications/resume-ir.app/desktop" },
        instanceId: "instance-0",
        launchId: "launch-0",
      },
      undefined,
      {
        async readDaemonConnection() {
          return {
            token: "token-0",
            instanceId: "instance-0",
            launchId: "launch-0",
          };
        },
        async nativeExport({ exportDirectory, fileName }) {
          const file = path.join(exportDirectory, fileName);
          await writeFile(
            file,
            `${JSON.stringify(combinedDiagnostics("included"))}\n`,
            { mode: 0o600 },
          );
          await chmod(file, 0o640);
        },
      },
    ),
    /desktop_diagnostics_export_file_invalid/,
  );
});
