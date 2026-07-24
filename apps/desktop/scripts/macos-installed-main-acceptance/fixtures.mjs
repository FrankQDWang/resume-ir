import assert from "node:assert/strict";

import {
  OPTIONAL_RUNTIME_NAMES,
  REQUIRED_FAULT_CELLS,
  runtimeFaultCase,
} from "./native-runtime-fault-plan.mjs";
import { SYNTHETIC_CANARY_FILE_NAME } from "./synthetic-canary.mjs";

export const HEAD = "a".repeat(40);
export const COMPOSITION = "b".repeat(64);
export const ICON = "c".repeat(64);
export const DMG = "d".repeat(64);
export const SOURCE = Object.freeze({
  authority: "exact_main_commit",
  base_commit: HEAD,
  source_tree_sha256: "e".repeat(64),
});

function faultCapabilities(expectedReasons) {
  const available = { state: "available", reason: null };
  const unavailable = (reason) => ({ state: "unavailable", reason });
  const embedding = Object.hasOwn(expectedReasons, "embedding");
  const ocr = Object.hasOwn(expectedReasons, "ocr");
  const classifier = Object.hasOwn(expectedReasons, "classifier");
  return {
    keyword_search: available,
    detail: available,
    semantic_search: embedding
      ? unavailable("embedding_unavailable")
      : available,
    hybrid_search: embedding
      ? { state: "degraded", reason: "embedding_unavailable" }
      : available,
    text_import: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : available,
    ocr_import: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : ocr
          ? unavailable("ocr_unavailable")
          : available,
    index_publication: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : available,
  };
}

function faultOptionalRuntimes(expectedReasons) {
  return Object.fromEntries(
    OPTIONAL_RUNTIME_NAMES.map((runtimeName) => [
      runtimeName,
      Object.hasOwn(expectedReasons, runtimeName)
        ? { state: "unavailable", reason: expectedReasons[runtimeName] }
        : { state: "available", reason: null },
    ]),
  );
}

function canonicalBehaviorRuntime(definition) {
  return OPTIONAL_RUNTIME_NAMES.find(
    (runtimeName) => definition?.cell === `${runtimeName}_missing`,
  );
}

export function options(overrides = {}) {
  return {
    authorizedSourceDataDir: "/synthetic/private/source",
    repoRoot: "/synthetic/repo",
    temporaryParent: "/synthetic/tmp",
    ...overrides,
  };
}

export function diagnostics(overrides = {}) {
  return {
    schema_version: "resume-ir.diagnostics.v4",
    privacy_boundary: "redacted_local_aggregate",
    contains_raw_resume_text: false,
    contains_queries: false,
    contains_resume_paths: false,
    contains_candidate_results: false,
    contains_snippet_text: false,
    visible_epoch: 7,
    evidence_lane: "gui_manual",
    evidence_status: "unaccepted",
    process_state: "ready",
    core: { state: "ready", reason: null },
    optional_runtimes: {
      embedding: { state: "available", reason: null },
      ocr: { state: "available", reason: null },
      classifier: { state: "available", reason: null },
    },
    capabilities: {
      keyword_search: { state: "available", reason: null },
      detail: { state: "available", reason: null },
      semantic_search: { state: "available", reason: null },
      hybrid_search: { state: "available", reason: null },
      text_import: { state: "available", reason: null },
      ocr_import: { state: "available", reason: null },
      index_publication: { state: "available", reason: null },
    },
    repair_progress: null,
    error: null,
    metrics: {
      ipc: {
        accepted: 4,
        completed: 4,
        client_disconnect: 0,
        request_failure: 0,
        response_failure: 0,
      },
      indexed_documents: 4,
      searchable_documents: 4,
      partial_documents: 0,
      ocr_queue_depth: 0,
      embedding_queue_depth: 0,
      recovery_queue_depth: 0,
      import_tasks_queued: 0,
      import_tasks_recoverable: 0,
      import_tasks_cancelled: 0,
      query_latency: {
        sample_count: 0,
        p50_ms: null,
        p95_ms: null,
        p99_ms: null,
        last_result_count: null,
      },
    },
    error_counts: {
      failed_retryable: 0,
      failed_permanent: 0,
      import_scan_errors: 0,
      ocr_page_budget_blocked: 0,
      ocr_language_unavailable: 0,
      scan_error_buckets: [],
    },
    benchmark_refs: [],
    ...overrides,
  };
}

export function fakeRuntime({
  failAt,
  cleanupFails = false,
  bindingOverrides = {},
  deploymentOverrides = {},
} = {}) {
  const calls = [];
  const faultCells = [];
  const faultByClone = new WeakMap();
  let cloneNumber = 0;
  let sessionNumber = 0;
  const runtime = {
    calls,
    async requireInstalledFaultHarness() {
      calls.push(["fault-harness"]);
    },
    async validatePreLockInputs() {
      calls.push(["preflight"]);
    },
    async precheckSourceAuthority() {
      calls.push(["precheck-source-authority"]);
      if (failAt === "source-provenance") {
        throw new Error("git_main_binding_invalid");
      }
      return { gitHead: HEAD };
    },
    async recoverInterruptedRuns() {
      calls.push(["recover-interrupted"]);
    },
    async verifySourceBeforeDeployment() {
      calls.push(["verify-source-before-deployment"]);
      if (failAt === "source-provenance") {
        throw new Error("git_main_binding_invalid");
      }
      return { gitHead: HEAD };
    },
    async bindSourceAuthorityAfterLease() {
      calls.push(["bind-source-authority-after-lease"]);
      if (failAt === "source-recheck") {
        throw new Error("source_authority_changed");
      }
      return { gitHead: HEAD };
    },
    async prepareInstalledRelease() {
      calls.push(["prepare-release"]);
      return {
        compositionDigest: COMPOSITION,
        deploymentAction: "reinstall",
        dmgSha256: DMG,
        gitHead: HEAD,
        source: SOURCE,
        version: "0.1.2",
        ...deploymentOverrides,
      };
    },
    async acquireAcceptanceLease() {
      calls.push(["acceptance-lock"]);
    },
    async verifyBindings() {
      calls.push(["verify"]);
      return {
        composition: { composition_digest: COMPOSITION },
        dmgSha256: DMG,
        gitHead: HEAD,
        iconSha256: ICON,
        source: SOURCE,
        sourceSchema: 29,
        version: "0.1.2",
        ...bindingOverrides,
      };
    },
    async createClone(label) {
      calls.push(["clone", label]);
      cloneNumber += 1;
      return { id: cloneNumber, label };
    },
    async launchApp(clone) {
      sessionNumber += 1;
      const session = { id: sessionNumber, clone };
      calls.push(["launch", clone.label, session.id]);
      return session;
    },
    async prepareStaleControl(clone) {
      calls.push(["prepare-stale-control", clone.label]);
      return { kind: "stale", clone };
    },
    async validateStaleControl(session) {
      calls.push(["validate-stale-control", session.id]);
      return {
        legacyContractReplaced: true,
        newGenerationReady: true,
        v29AuthorityPreserved: true,
      };
    },
    async prepareForeignControl(clone) {
      calls.push(["prepare-foreign-control", clone.label]);
      return { kind: "foreign", clone };
    },
    async validateForeignControl(session) {
      calls.push(["validate-foreign-control", session.id]);
      return {
        foreignEndpointPreserved: true,
        newGenerationReady: true,
        notAdopted: true,
        notProbed: true,
        v29AuthorityPreserved: true,
      };
    },
    async closeControlFixture(fixture) {
      calls.push(["close-control-fixture", fixture.kind]);
    },
    async prepareFaultCell(clone, cell) {
      calls.push(["prepare-fault", clone.label, cell]);
      const fault = {
        activated: false,
        behaviors: new Set(),
        cell,
        clone,
        definition: cell === "slow_initialization" ? null : runtimeFaultCase(cell),
        released: false,
        validated: false,
      };
      faultCells.push(fault);
      faultByClone.set(clone, fault);
      return fault;
    },
    async activateFaultCell(clone) {
      const fault = faultByClone.get(clone);
      if (!fault) return;
      fault.activated = true;
      calls.push(["activate-fault", clone.label, fault.cell]);
    },
    async validateSlowInitialization(session) {
      calls.push(["validate-slow-initialization", session.id]);
      faultByClone.get(session.clone).validated = true;
      return {
        sameInstance: true,
        sameLaunch: true,
        sameListener: true,
        slowWindowObserved: true,
        statusWithinTenSeconds: true,
        v29AuthorityPreserved: true,
      };
    },
    async validateRuntimeFaultCase(session, cell) {
      calls.push(["validate-runtime-fault", session.id, cell]);
      const fault = faultByClone.get(session.clone);
      assert.equal(fault.cell, cell);
      fault.validated = true;
      return {
        cell,
        capabilities: faultCapabilities(fault.definition.expectedReasons),
        coreState: "ready",
        evidenceSource: "installed_app",
        optionalRuntimes: faultOptionalRuntimes(
          fault.definition.expectedReasons,
        ),
        v29AuthorityPreserved: true,
      };
    },
    validateProjectedRuntimeFault(cell) {
      calls.push(["validate-projected-runtime-fault", cell]);
      const fault = faultCells.find(
        (candidate) => candidate.cell === cell && !candidate.released,
      );
      assert.equal(fault.activated, true);
      fault.validated = true;
      return {
        cell,
        evidenceSource: "deterministic_contract_projection",
        expectedReasons: fault.definition.expectedReasons,
        nativeMutationApplied: false,
        projectionReason: "post_attestation_failure_surface_absent",
      };
    },
    async captureFaultWitness(session) {
      calls.push(["capture-fault-witness", session.id]);
      return {
        docId: "doc-synthetic",
        versionId: "version-synthetic",
        visibleEpoch: 7,
      };
    },
    async prepareOcrFaultFixture(clone) {
      calls.push(["prepare-ocr-fixture", clone.label]);
      return {
        request: {
          roots: ["/synthetic/ocr"],
          profile: "explicit",
          max_files: 1,
        },
      };
    },
    async validateEmbeddingFaultBehavior(session, selection, cell) {
      calls.push(["validate-embedding-behavior", session.id, cell]);
      assert.equal(selection.docId, "doc-synthetic");
      faultByClone.get(session.clone).behaviors.add("embedding");
      return {
        detailAvailable: true,
        hybridLexicalPartial: true,
        keywordAvailable: true,
        selectionPreserved: true,
      };
    },
    async validateOcrFaultBehavior(session, fixture, cell) {
      calls.push(["validate-ocr-behavior", session.id, cell]);
      assert.equal(fixture.request.max_files, 1);
      faultByClone.get(session.clone).behaviors.add("ocr");
      return {
        backlogRetained: true,
        claimGateStable: true,
        visibleEpochPreserved: true,
      };
    },
    async validateClassifierFaultBehavior(session, request, cell) {
      calls.push(["validate-classifier-behavior", session.id, cell]);
      assert.deepEqual(request.roots, ["/synthetic/canary"]);
      faultByClone.get(session.clone).behaviors.add("classifier");
      return {
        classifierEpochPreserved: true,
        importRejectedBeforeClaim: true,
        visibleEpochPreserved: true,
      };
    },
    async releaseFaultCell(fault) {
      fault.released = true;
      calls.push(["release-fault", fault.cell]);
    },
    validateFaultCoverage() {
      calls.push(["fault-coverage"]);
      assert.deepEqual(
        faultCells.map(
          ({ activated, behaviors, cell, definition, released, validated }) => ({
            activated,
            behaviors: [...behaviors].sort(),
            cell,
            released,
            validated,
            expectedBehaviors: canonicalBehaviorRuntime(definition)
              ? [canonicalBehaviorRuntime(definition)]
              : [],
          }),
        ),
        REQUIRED_FAULT_CELLS.map((cell) => {
          const definition =
            cell === "slow_initialization" ? null : runtimeFaultCase(cell);
          const canonicalBehavior = canonicalBehaviorRuntime(definition);
          const expectedBehaviors = canonicalBehavior
            ? [canonicalBehavior]
            : [];
          return {
            activated: true,
            behaviors: expectedBehaviors,
            cell,
            released: true,
            validated: true,
            expectedBehaviors,
          };
        }),
      );
    },
    async waitForColdReady(session) {
      calls.push(["cold-ready", session.id]);
      if (failAt === "cold") throw new Error("/private/path raw stderr");
      return { instanceId: "cold-generation", v29AuthorityPreserved: true };
    },
    async validateColdReadyArtifacts(session, observed) {
      calls.push(["cold-artifacts", session.id, observed.instanceId]);
      return { generationAgreement: true, metadataArtifactBound: true };
    },
    async createSyntheticCanary(clone) {
      calls.push(["create-canary", clone.label]);
      return {
        file: `/synthetic/canary/${SYNTHETIC_CANARY_FILE_NAME}`,
        root: "/synthetic/canary",
      };
    },
    async importSyntheticCanary(session, canary, observed) {
      assert.equal(canary.root, "/synthetic/canary");
      calls.push(["import-canary", session.id, observed.instanceId]);
      return { instanceId: "canary-generation" };
    },
    async findOwnedDaemon(session) {
      calls.push(["find-daemon", session.id]);
      return { pid: 47_111 };
    },
    async strongKillDaemon(session, target) {
      calls.push(["strong-kill", session.id, target.pid]);
    },
    async captureRecoveryBoundary(session) {
      calls.push(["capture-recovery-boundary", session.id]);
      return { generation: 4 };
    },
    async waitForNewGenerationReady(session, oldInstance) {
      calls.push(["new-generation", session.id, oldInstance]);
      return { instanceId: "new-generation" };
    },
    async validateRecoveryBoundary(session, boundary) {
      calls.push(["validate-recovery-boundary", session.id, boundary.generation]);
    },
    async validateRecoveryEvidence(session, observed, canary) {
      assert.equal(canary.root, "/synthetic/canary");
      calls.push(["recovery-evidence", session.id, observed.instanceId]);
      return {
        generationAgreement: true,
        metadataArtifactBound: true,
        searchWitness: true,
      };
    },
    async quitApp(session) {
      calls.push(["quit", session.id]);
    },
    async assertZeroResidue(session) {
      calls.push(["zero-residue", session.id]);
    },
    async waitForReady(session) {
      calls.push(["ready", session.id]);
      return { instanceId: `ready-${session.id}` };
    },
    async verifyCombinedDiagnosticsExport(session) {
      calls.push(["combined-diagnostics", session.id]);
      return {
        desktopContract: "resume-ir.desktop-diagnostics.v2",
        nativeSaveDialog: true,
        ownerOnlyFile: true,
        boundedBytes: 256 * 1024,
        daemonAvailableState: "included",
        daemonUnavailableState: "unavailable",
        daemonDownLifecycleState: "circuit_open",
      };
    },
    async holdPublicationLock(clone, kind) {
      calls.push(["lock", clone.label, kind]);
      return { kind, clone };
    },
    async waitForContention(session, kind) {
      calls.push(["contention", session.id, kind]);
      return {
        attempts: [1, 2],
        backoffObservedMs: 1_000,
        errorKind: `${kind}_publication_busy`,
        instanceId: `${kind}-generation`,
      };
    },
    async releasePublicationLock(lock) {
      calls.push(["unlock", lock.clone.label, lock.kind]);
    },
    async waitForSameGenerationReady(session, instanceId) {
      calls.push(["same-generation-ready", session.id, instanceId]);
    },
    async waitForPersistentBlock(session, kind) {
      calls.push(["persistent-block", session.id, kind]);
      assert.equal(kind, "fulltext");
      return {
        action: "repair_required",
        attempt: 5,
        errorKind: "fulltext_publication_busy",
        instanceId: `${kind}-persistent-generation`,
        phase: "blocked",
        repairReason: "runtime_invariant",
      };
    },
    async cleanup() {
      calls.push(["cleanup"]);
      if (cleanupFails) throw new Error("/private/cleanup/path");
    },
  };
  return runtime;
}
