import {
  CONTROL_PUBLICATION_TIMEOUT_MS,
  READY_TIMEOUT_MS,
  SLOW_INITIALIZATION_MIN_MS,
  exactKeys,
  fail,
} from "./core.mjs";
import { SYNTHETIC_CANARY_TOKEN } from "./acceptance-evidence.mjs";
import { readActiveStoreManifest } from "./filesystem-cow.mjs";
import {
  initializingStatus,
  readyStatus,
  requestJson,
  validateDaemonDiagnostics,
} from "./ipc-contracts.mjs";
import {
  prepareForeignControlFixture,
  prepareStaleControlFixture,
  validateForeignControlPreserved,
  waitForControlReplacement,
} from "./native-control-cells.mjs";
import { REQUIRED_FAULT_CELLS } from "./native-fault-harness.mjs";
import {
  OPTIONAL_RUNTIME_NAMES,
  projectedRuntimeFaultEvidence,
  runtimeFaultCase,
  runtimeFaultStatusMatches,
} from "./native-runtime-fault-plan.mjs";
import {
  connectionBelongsToOwnedDaemon,
  pollStatus,
} from "./process-lifecycle.mjs";
import { createRuntimeFaultBehaviorCells } from "./native-runtime-fault-behavior.mjs";

function requireFaultHarness(harness) {
  if (
    !exactKeys(harness, [
      "activate",
      "prepare",
      "recover",
      "restore",
      "supportedCells",
    ]) ||
    !Array.isArray(harness.supportedCells) ||
    JSON.stringify(harness.supportedCells) !==
      JSON.stringify(REQUIRED_FAULT_CELLS) ||
    typeof harness.prepare !== "function" ||
    typeof harness.activate !== "function" ||
    typeof harness.recover !== "function" ||
    typeof harness.restore !== "function"
  ) {
    fail("installed_fault_harness_unavailable");
  }
  return harness;
}

async function v29AuthorityPreserved(session) {
  const preserved = await readActiveStoreManifest(session.dataDir);
  const authority = session.workspace?.v29Authority;
  return (
    preserved.schema === 29 &&
    authority?.schema === 29 &&
    preserved.fileName === authority.fileName &&
    preserved.digest === authority.digest
  );
}

function capabilityEvidence(status) {
  return Object.fromEntries(
    [
      "keyword_search",
      "detail",
      "semantic_search",
      "hybrid_search",
      "text_import",
      "ocr_import",
      "index_publication",
    ].map((name) => [name, { ...status.capabilities[name] }]),
  );
}

function diagnosticsForbidden(options, session, connection) {
  return [
    options.authorizedSourceDataDir,
    session.dataDir,
    session.home,
    connection.token,
    connection.instanceId,
    connection.launchId,
    SYNTHETIC_CANARY_TOKEN,
  ];
}

export function createNativeAcceptanceCells({
  faultHarness,
  getBindings,
  now,
  options,
  requireMutationAuthority,
  runTool,
  signal,
}) {
  const controlFixtures = [];
  const faultCells = [];
  const faultCellsByWorkspace = new WeakMap();

  async function closeControlFixture(fixture) {
    if (fixture.closed) return;
    await fixture.close?.();
    fixture.closed = true;
  }

  async function releaseFaultCell(cell, requireCompleted) {
    if (cell.released) return;
    await requireFaultHarness(faultHarness).restore(cell.handle, {
      requireCompleted,
    });
    cell.released = true;
  }

  function faultCellForSession(session, expectedCell) {
    const cell = faultCellsByWorkspace.get(session.workspace);
    if (
      !cell ||
      cell.cell !== expectedCell ||
      !cell.activated ||
      cell.released
    ) {
      fail("installed_fault_not_activated");
    }
    return cell;
  }

  async function ownedReplacement(session, fixture) {
    return waitForControlReplacement(
      fixture,
      (connection) =>
        connectionBelongsToOwnedDaemon(session, connection, runTool),
      signal,
    );
  }

  async function validateRuntimeFaultCase(session, expectedCell) {
    const faultCell = faultCellForSession(session, expectedCell);
    const definition = faultCell.definition;
    if (definition.evidenceSource !== "installed_app") {
      fail("optional_runtime_fault_evidence_invalid");
    }
    const observed = await pollStatus(
      session,
      (status) => runtimeFaultStatusMatches(status, definition),
      READY_TIMEOUT_MS,
      null,
      signal,
      runTool,
    );
    const diagnostics = await requestJson(
      observed.connection.urls.diagnostics,
      observed.connection.token,
      undefined,
      signal,
    );
    validateDaemonDiagnostics(
      diagnostics,
      diagnosticsForbidden(options, session, observed.connection),
    );
    if (
      JSON.stringify(diagnostics.core) !==
        JSON.stringify(observed.status.core) ||
      JSON.stringify(diagnostics.optional_runtimes) !==
        JSON.stringify(observed.status.optional_runtimes) ||
      JSON.stringify(diagnostics.capabilities) !==
        JSON.stringify(observed.status.capabilities) ||
      !(await v29AuthorityPreserved(session))
    ) {
      fail("optional_runtime_fault_evidence_invalid");
    }
    faultCell.validated = true;
    return {
      cell: definition.cell,
      capabilities: capabilityEvidence(observed.status),
      coreState: observed.status.core.state,
      evidenceSource: definition.evidenceSource,
      optionalRuntimes: Object.fromEntries(
        OPTIONAL_RUNTIME_NAMES.map((name) => [
          name,
          { ...observed.status.optional_runtimes[name] },
        ]),
      ),
      v29AuthorityPreserved: true,
    };
  }

  const runtimeFaultBehavior = createRuntimeFaultBehaviorCells({
    faultCellForSession,
    options,
    requireMutationAuthority,
    runTool,
    signal,
  });

  return {
    ...runtimeFaultBehavior,
    requireFaultHarness() {
      requireFaultHarness(faultHarness);
    },
    async recoverFaults() {
      await requireMutationAuthority();
      await requireFaultHarness(faultHarness).recover();
      await requireMutationAuthority();
    },
    async prepareStaleControl(workspace) {
      await requireMutationAuthority();
      const fixture = {
        ...(await prepareStaleControlFixture(workspace.dataDir)),
        closed: false,
      };
      controlFixtures.push(fixture);
      return fixture;
    },
    async prepareForeignControl(workspace) {
      await requireMutationAuthority();
      const fixture = {
        ...(await prepareForeignControlFixture(workspace.dataDir)),
        closed: false,
      };
      controlFixtures.push(fixture);
      return fixture;
    },
    async prepareFaultCell(workspace, cell) {
      await requireMutationAuthority();
      if (!REQUIRED_FAULT_CELLS.includes(cell)) {
        fail("installed_fault_cell_invalid");
      }
      if (
        workspace === null ||
        typeof workspace !== "object" ||
        faultCellsByWorkspace.has(workspace)
      ) {
        fail("installed_fault_activation_invalid");
      }
      const handle = await requireFaultHarness(faultHarness).prepare({
        cell,
        dataDir: workspace.dataDir,
        executablePaths: getBindings().executablePaths,
      });
      await requireMutationAuthority();
      const tracked = {
        activated: false,
        behaviorValidated: false,
        cell,
        definition:
          cell === "slow_initialization" ? null : runtimeFaultCase(cell),
        handle,
        released: false,
        validated: false,
        workspace,
      };
      faultCells.push(tracked);
      faultCellsByWorkspace.set(workspace, tracked);
      return tracked;
    },
    async activateFaultCell(workspace) {
      const cell = faultCellsByWorkspace.get(workspace);
      if (!cell) return;
      if (cell.activated || cell.released) {
        fail("installed_fault_activation_invalid");
      }
      await requireMutationAuthority();
      await requireFaultHarness(faultHarness).activate(cell.handle);
      cell.activated = true;
      await requireMutationAuthority();
    },
    assertWorkspaceLaunchable(workspace) {
      const cell = faultCellsByWorkspace.get(workspace);
      if (cell && (!cell.activated || cell.released)) {
        fail("installed_fault_not_activated");
      }
    },
    async validateStaleControl(session, fixture) {
      const replacement = await ownedReplacement(session, fixture);
      const ready = await pollStatus(
        session,
        readyStatus,
        READY_TIMEOUT_MS,
        replacement.instanceId,
        signal,
        runTool,
      );
      if (
        fixture.kind !== "stale" ||
        ready.launchId !== replacement.launchId ||
        ready.connection.urls.status.origin !==
          replacement.urls.status.origin ||
        !(await v29AuthorityPreserved(session))
      ) {
        fail("stale_control_evidence_invalid");
      }
      return {
        legacyContractReplaced: true,
        newGenerationReady: true,
        v29AuthorityPreserved: true,
      };
    },
    async validateForeignControl(session, fixture) {
      const replacement = await ownedReplacement(session, fixture);
      const ready = await pollStatus(
        session,
        readyStatus,
        READY_TIMEOUT_MS,
        replacement.instanceId,
        signal,
        runTool,
      );
      if (
        ready.launchId !== replacement.launchId ||
        ready.connection.urls.status.origin !==
          replacement.urls.status.origin ||
        !(await v29AuthorityPreserved(session))
      ) {
        fail("foreign_control_evidence_invalid");
      }
      validateForeignControlPreserved(fixture);
      return {
        foreignEndpointPreserved: true,
        newGenerationReady: true,
        notAdopted: true,
        notProbed: true,
        v29AuthorityPreserved: true,
      };
    },
    closeControlFixture,
    async validateSlowInitialization(session) {
      const faultCell = faultCellForSession(session, "slow_initialization");
      const initializing = await pollStatus(
        session,
        initializingStatus,
        CONTROL_PUBLICATION_TIMEOUT_MS,
        null,
        signal,
        runTool,
      );
      const initializingElapsed = now() - session.acceptanceStartedAt;
      const ready = await pollStatus(
        session,
        readyStatus,
        READY_TIMEOUT_MS,
        initializing.instanceId,
        signal,
        runTool,
      );
      const readyElapsed = now() - session.acceptanceStartedAt;
      if (
        !Number.isFinite(initializingElapsed) ||
        initializingElapsed < 0 ||
        initializingElapsed > CONTROL_PUBLICATION_TIMEOUT_MS ||
        !Number.isFinite(readyElapsed) ||
        readyElapsed < SLOW_INITIALIZATION_MIN_MS ||
        ready.instanceId !== initializing.instanceId ||
        ready.launchId !== initializing.launchId ||
        ready.connection.urls.status.origin !==
          initializing.connection.urls.status.origin ||
        !(await v29AuthorityPreserved(session))
      ) {
        fail("slow_initialization_evidence_invalid");
      }
      faultCell.validated = true;
      return {
        sameInstance: true,
        sameLaunch: true,
        sameListener: true,
        slowWindowObserved: true,
        statusWithinTenSeconds: true,
        v29AuthorityPreserved: true,
      };
    },
    validateRuntimeFaultCase,
    validateProjectedRuntimeFault(cell) {
      const tracked = faultCells.find(
        (candidate) => candidate.cell === cell && !candidate.released,
      );
      if (!tracked || !tracked.activated || tracked.validated) {
        fail("installed_fault_not_activated");
      }
      const evidence = projectedRuntimeFaultEvidence(cell);
      tracked.validated = true;
      return evidence;
    },
    async releaseFaultCell(cell) {
      await requireMutationAuthority();
      if (!cell.activated || !cell.validated) {
        fail("installed_fault_not_completed");
      }
      await releaseFaultCell(cell, true);
      await requireMutationAuthority();
    },
    validateFaultCoverage() {
      runtimeFaultBehavior.validateBehaviorCoverage();
      if (
        faultCells.length !== REQUIRED_FAULT_CELLS.length ||
        !REQUIRED_FAULT_CELLS.every((name) => {
          const matches = faultCells.filter(({ cell }) => cell === name);
          return (
            matches.length === 1 &&
            matches[0].activated &&
            matches[0].validated &&
            matches[0].released
          );
        })
      ) {
        fail("installed_fault_coverage_incomplete");
      }
    },
    async cleanup() {
      let failed = false;
      for (const cell of [...faultCells].reverse()) {
        try {
          await releaseFaultCell(cell, false);
        } catch {
          failed = true;
        }
      }
      for (const fixture of [...controlFixtures].reverse()) {
        try {
          await closeControlFixture(fixture);
        } catch {
          failed = true;
        }
      }
      if (failed) fail("cleanup_failed");
    },
  };
}
