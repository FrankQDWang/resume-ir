import assert from "node:assert/strict";

export const HEAD = "a".repeat(40);
export const COMPOSITION = "b".repeat(64);
export const ICON = "c".repeat(64);
export const DMG = "d".repeat(64);

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
    schema_version: "resume-ir.diagnostics.v3",
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
    service_state: "ready",
    services: { metadata: "ready", query: "ready" },
    repair_reason: null,
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
  let cloneNumber = 0;
  let sessionNumber = 0;
  const runtime = {
    calls,
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
        version: "0.1.2",
        ...deploymentOverrides,
      };
    },
    async acquireLifecycleLease() {
      calls.push(["lifecycle-lock"]);
    },
    async verifyBindings() {
      calls.push(["verify"]);
      return {
        composition: { composition_digest: COMPOSITION },
        dmgSha256: DMG,
        gitHead: HEAD,
        iconSha256: ICON,
        sourceSchema: 28,
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
    async waitForColdReady(session) {
      calls.push(["cold-ready", session.id]);
      if (failAt === "cold") throw new Error("/private/path raw stderr");
      return { instanceId: "cold-generation" };
    },
    async validateColdReadyArtifacts(session, observed) {
      calls.push(["cold-artifacts", session.id, observed.instanceId]);
      return { generationAgreement: true, metadataArtifactBound: true };
    },
    async createSyntheticCanary(clone) {
      calls.push(["create-canary", clone.label]);
      return { file: "/synthetic/canary/file", root: "/synthetic/canary" };
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
    async validateDiagnostics(session) {
      calls.push(["diagnostics", session.id]);
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
