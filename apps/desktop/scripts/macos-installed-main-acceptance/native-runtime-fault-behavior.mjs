import { READY_TIMEOUT_MS, fail, wait } from "./core.mjs";
import {
  readyStatus,
  requestJson,
} from "./ipc-contracts.mjs";
import { pollStatus } from "./process-lifecycle.mjs";
import {
  captureSyntheticFaultWitness,
  createOcrFaultFixture,
  readActiveClassifierEpoch,
  submitImportExpectedRejected,
  submitOcrBacklogImport,
  validateEmbeddingFaultDataPlane,
} from "./native-runtime-fault-evidence.mjs";
import { runtimeFaultStatusMatches } from "./native-runtime-fault-plan.mjs";

const GATE_STABILITY_MS = 750;

export function createRuntimeFaultBehaviorCells({
  faultCellForSession,
  options,
  requireMutationAuthority,
  runTool,
  signal,
}) {
  const covered = {
    classifier: false,
    embedding: false,
    ocr: false,
  };

  async function observedFault(session, expectedCell) {
    const tracked = faultCellForSession(session, expectedCell);
    if (tracked.definition.evidenceSource !== "installed_app") {
      fail("optional_runtime_fault_evidence_invalid");
    }
    const observed = await pollStatus(
      session,
      (status) => runtimeFaultStatusMatches(status, tracked.definition),
      READY_TIMEOUT_MS,
      null,
      signal,
      runTool,
    );
    return { observed, tracked };
  }

  return Object.freeze({
    async captureFaultWitness(session) {
      const observed = await pollStatus(
        session,
        readyStatus,
        READY_TIMEOUT_MS,
        null,
        signal,
        runTool,
      );
      return captureSyntheticFaultWitness(observed.connection, signal);
    },
    async prepareOcrFaultFixture(workspace) {
      await requireMutationAuthority();
      const fixture = await createOcrFaultFixture(workspace, options.repoRoot);
      await requireMutationAuthority();
      return fixture;
    },
    async validateEmbeddingFaultBehavior(
      session,
      expectedSelection,
      expectedCell = "embedding_missing",
    ) {
      const { observed, tracked } = await observedFault(session, expectedCell);
      const evidence = await validateEmbeddingFaultDataPlane(
        observed.connection,
        expectedSelection,
        signal,
      );
      tracked.behaviorValidated = true;
      covered.embedding = true;
      return evidence;
    },
    async validateOcrFaultBehavior(
      session,
      fixture,
      expectedCell = "ocr_missing",
    ) {
      const { observed: before, tracked } = await observedFault(
        session,
        expectedCell,
      );
      await submitOcrBacklogImport(
        before.connection,
        fixture.request,
        signal,
      );
      const queued = await pollStatus(
        session,
        (status) =>
          runtimeFaultStatusMatches(status, tracked.definition) &&
          status.ocr_jobs_queued > before.status.ocr_jobs_queued &&
          status.ocr_queue_depth > before.status.ocr_queue_depth &&
          status.latest_import_scan?.ocr_required_documents === 1,
        READY_TIMEOUT_MS,
        before.instanceId,
        signal,
        runTool,
      );
      await wait(GATE_STABILITY_MS);
      const stable = await requestJson(
        queued.connection.urls.status,
        queued.connection.token,
        undefined,
        signal,
      );
      if (
        !runtimeFaultStatusMatches(stable, tracked.definition) ||
        stable.visible_epoch !== before.status.visible_epoch ||
        stable.ocr_jobs_queued !== queued.status.ocr_jobs_queued ||
        stable.ocr_queue_depth !== queued.status.ocr_queue_depth
      ) {
        fail("optional_runtime_claim_gate_invalid");
      }
      tracked.behaviorValidated = true;
      covered.ocr = true;
      return Object.freeze({
        backlogRetained: true,
        claimGateStable: true,
        visibleEpochPreserved: true,
      });
    },
    async validateClassifierFaultBehavior(
      session,
      importRequest,
      expectedCell = "classifier_missing",
    ) {
      const { observed: before, tracked } = await observedFault(
        session,
        expectedCell,
      );
      const beforeEpoch = await readActiveClassifierEpoch(
        session.dataDir,
        runTool,
      );
      await submitImportExpectedRejected(
        before.connection,
        importRequest,
        signal,
      );
      await wait(GATE_STABILITY_MS);
      const stable = await requestJson(
        before.connection.urls.status,
        before.connection.token,
        undefined,
        signal,
      );
      const afterEpoch = await readActiveClassifierEpoch(
        session.dataDir,
        runTool,
      );
      if (
        !runtimeFaultStatusMatches(stable, tracked.definition) ||
        stable.visible_epoch !== before.status.visible_epoch ||
        stable.import_tasks_queued !== before.status.import_tasks_queued ||
        stable.import_tasks_recoverable !== before.status.import_tasks_recoverable ||
        afterEpoch !== beforeEpoch
      ) {
        fail("optional_runtime_claim_gate_invalid");
      }
      tracked.behaviorValidated = true;
      covered.classifier = true;
      return Object.freeze({
        classifierEpochPreserved: true,
        importRejectedBeforeClaim: true,
        visibleEpochPreserved: true,
      });
    },
    validateBehaviorCoverage() {
      if (!Object.values(covered).every(Boolean)) {
        fail("installed_fault_behavior_coverage_incomplete");
      }
    },
  });
}
