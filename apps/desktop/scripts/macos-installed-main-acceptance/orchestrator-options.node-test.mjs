import assert from "node:assert/strict";
import { realpathSync } from "node:fs";
import os from "node:os";
import test from "node:test";

import { ACCEPTANCE_SCHEMA, AcceptanceError } from "./core.mjs";
import { validateDaemonDiagnostics } from "./ipc-contracts.mjs";
import { normalizeAcceptanceOptions, parseAcceptanceArgs } from "./options.mjs";
import {
  createNativeAcceptanceRuntime,
  observedRealBackoff,
  runInstalledMainAcceptanceForTesting as runInstalledMainAcceptance,
} from "./orchestrator-receipt.mjs";
import { HEAD, diagnostics, fakeRuntime, options } from "./fixtures.mjs";
import {
  OPTIONAL_RUNTIME_NAMES,
  RUNTIME_FAULT_CASES,
} from "./native-runtime-fault-plan.mjs";

function expectedRuntimeFaultCalls(firstSessionId) {
  const calls = [];
  let sessionId = firstSessionId;
  for (const definition of RUNTIME_FAULT_CASES) {
    const { cell, evidenceSource } = definition;
    calls.push(["clone", cell]);
    if (evidenceSource === "deterministic_contract_projection") {
      calls.push(
        ["prepare-fault", cell, cell],
        ["activate-fault", cell, cell],
        ["validate-projected-runtime-fault", cell],
        ["release-fault", cell],
      );
      continue;
    }
    const behaviorRuntime = OPTIONAL_RUNTIME_NAMES.find(
      (runtimeName) => cell === `${runtimeName}_missing`,
    );
    if (behaviorRuntime === "embedding") {
      calls.push(
        ["verify"],
        ["launch", cell, sessionId],
        ["ready", sessionId],
        ["create-canary", cell],
        ["import-canary", sessionId, `ready-${sessionId}`],
        ["capture-fault-witness", sessionId],
        ["quit", sessionId],
        ["zero-residue", sessionId],
      );
      sessionId += 1;
    } else if (behaviorRuntime === "classifier") {
      calls.push(["create-canary", cell]);
    }
    if (behaviorRuntime === "ocr") {
      calls.push(["prepare-ocr-fixture", cell]);
    }
    calls.push(
      ["prepare-fault", cell, cell],
      ["verify"],
      ["activate-fault", cell, cell],
      ["launch", cell, sessionId],
      ["validate-runtime-fault", sessionId, cell],
    );
    if (behaviorRuntime) {
      calls.push([
        `validate-${behaviorRuntime}-behavior`,
        sessionId,
        cell,
      ]);
    }
    calls.push(
      ["quit", sessionId],
      ["zero-residue", sessionId],
      ["release-fault", cell],
    );
    sessionId += 1;
  }
  return { calls, nextSessionId: sessionId };
}

test("accepts only real source, repository, and temporary-root inputs", () => {
  assert.throws(
    () => normalizeAcceptanceOptions({}),
    (error) =>
      error instanceof AcceptanceError && error.code === "arguments_invalid",
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        diagnostics({ private_debug: "raw resume and query text" }),
      ),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () => parseAcceptanceArgs(["--data-dir", "/forbidden"]),
    /arguments_invalid/,
  );

  const parsed = parseAcceptanceArgs([
    "--authorized-source-data-dir",
    "/synthetic/private/source",
    "--repo-root",
    "/synthetic/repo",
    "--temporary-parent",
    "/synthetic/tmp",
  ]);
  assert.equal(parsed.authorizedSourceDataDir, "/synthetic/private/source");
  assert.equal(parsed.repoRoot, "/synthetic/repo");
  assert.equal(parsed.temporaryParent, "/synthetic/tmp");
  for (const [name, value] of [
    ["--expected-git-head", HEAD],
    ["--expected-version", "0.1.2"],
    ["--expected-composition-digest", "b".repeat(64)],
    ["--expected-icon-sha256", "c".repeat(64)],
    ["--persistent-contention", "fulltext"],
  ]) {
    assert.throws(
      () =>
        parseAcceptanceArgs([
          "--authorized-source-data-dir",
          "/synthetic/private/source",
          name,
          value,
        ]),
      /arguments_invalid/,
    );
  }

  const defaulted = normalizeAcceptanceOptions(
    options({ temporaryParent: undefined }),
  );
  assert.equal(defaulted.temporaryParent, realpathSync(os.tmpdir()));
});

test("orchestrates exact deployment, cold recovery, ordered contention, and the final relaunch", async () => {
  const runtime = fakeRuntime();
  const report = await runInstalledMainAcceptance(options(), { runtime });

  assert.equal(report.schema_version, ACCEPTANCE_SCHEMA);
  assert.equal(report.outcome, "not_native_evidence");
  assert.equal(report.evidence_scope, "dependency_injected_structure_only");
  assert.equal(report.bindings.git_head, HEAD);
  assert.deepEqual(report.deployment, {
    action: "reinstall",
    built_dmg_verified: true,
    installed_version: "0.1.2",
    source: "clean_origin_main",
  });
  assert.equal(report.data_boundary.clone, "apfs_copy_on_write");
  assert.equal(report.data_boundary.source_mutated, false);
  assert.equal(
    report.data_boundary.acceptance_lock,
    "held_for_entire_acceptance",
  );
  assert.equal(
    report.data_boundary.lifecycle_lock,
    "owned_by_install_children",
  );
  assert.equal(report.data_boundary.release_data_dir_override, false);
  assert.equal(report.supervised_strong_kill.targeting, "exact_owned_child");
  assert.equal(report.normal_quit_relaunch.process_residue, "none");
  assert.equal(
    report.bootstrap_control.foreign.foreign_endpoint_preserved,
    true,
  );
  assert.equal(
    report.bootstrap_control.slow_initialization.same_listener,
    true,
  );
  assert.equal(
    report.optional_runtime_faults.embedding_missing.native_observation
      .capabilities.hybrid_search.state,
    "degraded",
  );
  assert.equal(
    report.optional_runtime_faults.ocr_missing.native_observation.capabilities
      .ocr_import.reason,
    "ocr_unavailable",
  );
  assert.equal(
    report.optional_runtime_faults.classifier_missing.native_observation
      .capabilities.text_import.reason,
    "classifier_unavailable",
  );
  const runtimeFaultSequence = expectedRuntimeFaultCalls(8);
  assert.deepEqual(
    Object.keys(report.optional_runtime_faults),
    RUNTIME_FAULT_CASES.map(({ cell }) => cell),
  );
  assert.equal(
    report.optional_runtime_faults.embedding_missing.behavior_evidence
      .embedding.hybrid_lexical_partial,
    true,
  );
  assert.equal(
    report.optional_runtime_faults.ocr_missing.behavior_evidence.ocr
      .backlog_retained,
    true,
  );
  assert.equal(
    report.optional_runtime_faults.classifier_missing.behavior_evidence
      .classifier.classifier_epoch_preserved,
    true,
  );
  assert.deepEqual(
    report.optional_runtime_faults.embedding_classifier_invalid
      .behavior_evidence,
    {},
  );
  assert.equal(
    report.optional_runtime_faults.embedding_classifier_invalid
      .validation_scope,
    "status_capability_matrix_only",
  );
  assert.equal(
    report.optional_runtime_faults.embedding_classifier_invalid
      .native_observation.capabilities.text_import.reason,
    "classifier_unavailable",
  );
  assert.equal(
    report.optional_runtime_faults.all_runtimes_missing.native_observation
      .capabilities.ocr_import.reason,
    "classifier_unavailable",
  );
  assert.deepEqual(report.optional_runtime_faults.classifier_start_failed, {
    evidence_source: "deterministic_contract_projection",
    expected_runtime_reasons: { classifier: "start_failed" },
    native_mutation_applied: false,
    native_observation: null,
    behavior_evidence: null,
    projection_reason: "post_attestation_failure_surface_absent",
  });
  assert.deepEqual(Object.keys(report.contention), ["fulltext", "vector"]);
  assert.equal(report.contention.fulltext.daemon_restart, false);
  assert.equal(report.contention.vector.convergence, "ready");
  assert.deepEqual(report.contention.fulltext.attempts_observed, [1, 2]);
  assert.equal(report.contention.vector.error_kind, "vector_publication_busy");
  assert.equal(report.persistent_contention.attempt, 5);
  assert.equal(
    report.persistent_contention.error_kind,
    "fulltext_publication_busy",
  );
  assert.equal(report.persistent_contention.core_reason, "runtime_invariant");
  assert.equal(report.cold_start.v29_manifest_identity_preserved, true);
  assert.equal(report.cold_start.v29_logical_authority_preserved, true);
  assert.deepEqual(report.diagnostics.gui_combined_export, {
    desktop_contract: "resume-ir.desktop-diagnostics.v2",
    daemon_available_export: "verified",
    daemon_unavailable_export: "verified",
    daemon_down_lifecycle_state: "circuit_open",
    native_save_dialog: "automated",
    maximum_export_bytes: 256 * 1024,
    file_permissions: "owner_only",
  });
  assert.equal(report.cleanup, "temporary_clones_removed");

  const clones = runtime.calls
    .filter(([operation]) => operation === "clone")
    .map(([, label]) => label);
  assert.deepEqual(clones, [
    "cold-start",
    "fulltext-contention",
    "vector-contention",
    "fulltext-persistent-contention",
    "stale-control",
    "foreign-control",
    "slow-initialization",
    ...RUNTIME_FAULT_CASES.map(({ cell }) => cell),
  ]);
  assert.ok(
    runtime.calls.some(
      (call) =>
        JSON.stringify(call) === JSON.stringify(["strong-kill", 1, 47_111]),
    ),
  );
  assert.equal(runtime.calls.at(-1)[0], "cleanup");
  assert.deepEqual(runtime.calls.slice(0, 7), [
    ["preflight"],
    ["fault-harness"],
    ["precheck-source-authority"],
    ["acceptance-lock"],
    ["bind-source-authority-after-lease"],
    ["recover-interrupted"],
    ["prepare-release"],
  ]);
  for (let index = 0; index < runtime.calls.length; index += 1) {
    if (runtime.calls[index][0] === "launch") {
      const immediatelyBefore = runtime.calls[index - 1][0];
      if (immediatelyBefore === "activate-fault") {
        assert.equal(runtime.calls[index - 2][0], "verify");
      } else {
        assert.equal(immediatelyBefore, "verify");
      }
    }
  }
  const launchCount = runtime.calls.filter(
    ([operation]) => operation === "launch",
  ).length;
  assert.equal(
    runtime.calls.filter(([operation]) => operation === "verify").length,
    launchCount + 2,
  );
  assert.equal(runtime.calls.at(-2)[0], "verify");
  for (const kind of ["fulltext", "vector"]) {
    const observed = runtime.calls.findIndex(
      ([operation, , observedKind]) =>
        operation === "contention" && observedKind === kind,
    );
    const released = runtime.calls.findIndex(
      ([operation, , releasedKind]) =>
        operation === "unlock" && releasedKind === kind,
    );
    assert.ok(observed >= 0 && released > observed);
  }

  assert.deepEqual(
    runtime.calls.map(([operation, ...details]) =>
      operation === "verify" ? [operation] : [operation, ...details],
    ),
    [
      ["preflight"],
      ["fault-harness"],
      ["precheck-source-authority"],
      ["acceptance-lock"],
      ["bind-source-authority-after-lease"],
      ["recover-interrupted"],
      ["prepare-release"],
      ["verify"],
      ["clone", "cold-start"],
      ["verify"],
      ["launch", "cold-start", 1],
      ["cold-ready", 1],
      ["cold-artifacts", 1, "cold-generation"],
      ["create-canary", "cold-start"],
      ["import-canary", 1, "cold-generation"],
      ["recovery-evidence", 1, "canary-generation"],
      ["find-daemon", 1],
      ["capture-recovery-boundary", 1],
      ["strong-kill", 1, 47_111],
      ["new-generation", 1, "cold-generation"],
      ["validate-recovery-boundary", 1, 4],
      ["ready", 1],
      ["recovery-evidence", 1, "ready-1"],
      ["quit", 1],
      ["zero-residue", 1],
      ["clone", "fulltext-contention"],
      ["lock", "fulltext-contention", "fulltext"],
      ["verify"],
      ["launch", "fulltext-contention", 2],
      ["contention", 2, "fulltext"],
      ["unlock", "fulltext-contention", "fulltext"],
      ["same-generation-ready", 2, "fulltext-generation"],
      ["quit", 2],
      ["zero-residue", 2],
      ["clone", "vector-contention"],
      ["lock", "vector-contention", "vector"],
      ["verify"],
      ["launch", "vector-contention", 3],
      ["contention", 3, "vector"],
      ["unlock", "vector-contention", "vector"],
      ["same-generation-ready", 3, "vector-generation"],
      ["quit", 3],
      ["zero-residue", 3],
      ["clone", "fulltext-persistent-contention"],
      ["lock", "fulltext-persistent-contention", "fulltext"],
      ["verify"],
      ["launch", "fulltext-persistent-contention", 4],
      ["persistent-block", 4, "fulltext"],
      ["quit", 4],
      ["zero-residue", 4],
      ["unlock", "fulltext-persistent-contention", "fulltext"],
      ["clone", "stale-control"],
      ["prepare-stale-control", "stale-control"],
      ["verify"],
      ["launch", "stale-control", 5],
      ["validate-stale-control", 5],
      ["quit", 5],
      ["zero-residue", 5],
      ["close-control-fixture", "stale"],
      ["clone", "foreign-control"],
      ["prepare-foreign-control", "foreign-control"],
      ["verify"],
      ["launch", "foreign-control", 6],
      ["validate-foreign-control", 6],
      ["quit", 6],
      ["zero-residue", 6],
      ["close-control-fixture", "foreign"],
      ["clone", "slow-initialization"],
      ["prepare-fault", "slow-initialization", "slow_initialization"],
      ["verify"],
      ["activate-fault", "slow-initialization", "slow_initialization"],
      ["launch", "slow-initialization", 7],
      ["validate-slow-initialization", 7],
      ["quit", 7],
      ["zero-residue", 7],
      ["release-fault", "slow_initialization"],
      ...runtimeFaultSequence.calls,
      ["fault-coverage"],
      ["verify"],
      ["launch", "cold-start", runtimeFaultSequence.nextSessionId],
      ["ready", runtimeFaultSequence.nextSessionId],
      [
        "recovery-evidence",
        runtimeFaultSequence.nextSessionId,
        `ready-${runtimeFaultSequence.nextSessionId}`,
      ],
      ["combined-diagnostics", runtimeFaultSequence.nextSessionId],
      ["quit", runtimeFaultSequence.nextSessionId],
      ["zero-residue", runtimeFaultSequence.nextSessionId],
      ["verify"],
      ["cleanup"],
    ],
  );

  const publicBody = JSON.stringify(report);
  assert.equal(publicBody.includes("/synthetic/private/source"), false);
  assert.equal(publicBody.includes("raw stderr"), false);
  assert.equal(publicBody.includes("47111"), false);
});

test("cannot issue a passing receipt when any required native control or fault cell is absent", async () => {
  const cases = [
    ["stale", "validateStaleControl", null],
    ["foreign", "validateForeignControl", null],
    ["slow", "validateSlowInitialization", null],
    ["installed status", "validateRuntimeFaultCase", "embedding_invalid"],
    [
      "projected status",
      "validateProjectedRuntimeFault",
      "classifier_start_failed",
    ],
    [
      "embedding behavior",
      "validateEmbeddingFaultBehavior",
      "embedding_missing",
    ],
    ["ocr behavior", "validateOcrFaultBehavior", "ocr_missing"],
    [
      "classifier behavior",
      "validateClassifierFaultBehavior",
      "classifier_missing",
    ],
  ];
  for (const [label, method, selectedRuntime] of cases) {
    const runtime = fakeRuntime();
    const original = runtime[method].bind(runtime);
    runtime[method] = async (...args) => {
      if (
        selectedRuntime === null ||
        args.at(-1) === selectedRuntime
      ) {
        return undefined;
      }
      return original(...args);
    };
    await assert.rejects(
      runInstalledMainAcceptance(options(), { runtime }),
      /evidence_invalid/,
      label,
    );
  }
});

test("cannot issue a passing receipt without both verified native diagnostics exports", async () => {
  const runtime = fakeRuntime();
  runtime.verifyCombinedDiagnosticsExport = async () => ({
    desktopContract: "resume-ir.desktop-diagnostics.v2",
    nativeSaveDialog: true,
    ownerOnlyFile: true,
    boundedBytes: 256 * 1024,
    daemonAvailableState: "included",
    daemonUnavailableState: "included",
    daemonDownLifecycleState: "circuit_open",
  });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /combined_diagnostics_evidence_invalid/,
  );
  assert.equal(runtime.calls.at(-1)[0], "cleanup");
});

test("the default CLI runtime wires the real external fault harness without touching the App", async () => {
  const runtime = createNativeAcceptanceRuntime(options());
  await runtime.requireInstalledFaultHarness();
});

test("an unactivated fault cell cannot produce a passing receipt", async () => {
  const runtime = fakeRuntime();
  const activate = runtime.activateFaultCell.bind(runtime);
  runtime.activateFaultCell = async (clone) => {
    if (clone.label === "classifier_missing") return;
    await activate(clone);
  };
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /acceptance_internal_failure/,
  );
  assert.equal(runtime.calls.at(-1)[0], "cleanup");
});

test("source provenance is rechecked under the acceptance lease before recovery", async () => {
  const runtime = fakeRuntime({ failAt: "source-recheck" });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /acceptance_internal_failure/,
  );
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["fault-harness"],
    ["precheck-source-authority"],
    ["acceptance-lock"],
    ["bind-source-authority-after-lease"],
    ["cleanup"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) => operation === "prepare-release"),
    false,
  );
});

test("read-only provenance fails before acceptance acquisition or stale recovery", async () => {
  const runtime = fakeRuntime({ failAt: "source-provenance" });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /acceptance_internal_failure/,
  );
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["fault-harness"],
    ["precheck-source-authority"],
    ["cleanup"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) =>
      ["acceptance-lock", "recover-interrupted", "prepare-release"].includes(
        operation,
      ),
    ),
    false,
  );
});

test("a concurrent run that cannot acquire the acceptance lease never enters build", async () => {
  let rejectLease;
  const leaseBlocked = new Promise((_resolve, reject) => {
    rejectLease = reject;
  });
  const runtime = fakeRuntime();
  runtime.acquireAcceptanceLease = async () => {
    runtime.calls.push(["acceptance-lock"]);
    await leaseBlocked;
  };
  const running = runInstalledMainAcceptance(options(), { runtime });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["fault-harness"],
    ["precheck-source-authority"],
    ["acceptance-lock"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) => operation === "prepare-release"),
    false,
  );
  rejectLease(new Error("synthetic acceptance contention"));
  await assert.rejects(running, /acceptance_internal_failure/);
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["fault-harness"],
    ["precheck-source-authority"],
    ["acceptance-lock"],
    ["cleanup"],
  ]);
});

test("every runtime mutation entrypoint revalidates the live lease and source authority", async () => {
  let leaseValid = true;
  let observedHead = HEAD;
  const authority = () => ({
    detached: false,
    gitHead: observedHead,
  });
  const runtime = createNativeAcceptanceRuntime(options(), {
    acquireInstalledMainAcceptanceLock: async () => ({ synthetic: true }),
    prepareInstalledMainAcceptanceLockFile: async () =>
      "/synthetic/support/local.resume-ir.desktop/macos-installed-main-acceptance.lock",
    requireDefaultApplicationSupportRoot: async (value) => value,
    requireInstalledMainAcceptanceLockCapability: () => {
      if (!leaseValid) throw new Error("lease lost");
    },
    resolveApplicationSupportRoot: async () => "/synthetic/support",
    validatePreLockInputs: async () => ({
      authorizedSourceDataDir: "/synthetic/private/source",
      repoRoot: "/synthetic/repo",
      temporaryParent: "/synthetic/tmp",
    }),
    releaseDeploymentOverrides: {
      deriveCommitProductBinding: async () => ({
        iconSha256: "c".repeat(64),
        version: "0.1.2",
      }),
      verifyGitMainBinding: async () => authority(),
    },
  });
  await runtime.validatePreLockInputs();
  const expected = await runtime.precheckSourceAuthority();
  await runtime.acquireAcceptanceLease();
  await runtime.bindSourceAuthorityAfterLease(expected);

  const mutationAttempts = [
    () => runtime.recoverInterruptedRuns(),
    () => runtime.prepareInstalledRelease(),
    () => runtime.createClone("synthetic"),
    () => runtime.prepareStaleControl({}),
    () => runtime.prepareForeignControl({}),
    () => runtime.prepareFaultCell({}, "embedding_missing"),
    () => runtime.prepareOcrFaultFixture({}),
    () => runtime.launchApp({}),
    () => runtime.createSyntheticCanary({}),
    () =>
      runtime.importSyntheticCanary(
        {},
        {},
        { connection: {}, status: { visible_epoch: 1 } },
      ),
    () => runtime.strongKillDaemon({}, {}),
    () => runtime.quitApp({}),
    () => runtime.holdPublicationLock({}, "fulltext"),
    () => runtime.releasePublicationLock({}),
    () => runtime.releaseFaultCell({}),
  ];
  leaseValid = false;
  for (const mutate of mutationAttempts) {
    await assert.rejects(mutate(), /acceptance_lock_lost/);
  }

  leaseValid = true;
  observedHead = "f".repeat(40);
  await assert.rejects(
    runtime.createClone("synthetic"),
    /source_authority_changed/,
  );
});

test("rejects an installed App or receipt that did not come from this exact 0.1.2 build", async () => {
  await assert.rejects(
    runInstalledMainAcceptance(options(), {
      runtime: fakeRuntime({
        deploymentOverrides: { compositionDigest: "e".repeat(64) },
      }),
    }),
    /installed_deployment_binding_mismatch/,
  );
  await assert.rejects(
    runInstalledMainAcceptance(options(), {
      runtime: fakeRuntime({ bindingOverrides: { version: "0.1.3" } }),
    }),
    /installed_deployment_binding_mismatch/,
  );
});

test("requires the exact fulltext attempt-five blocked lane", async () => {
  const runtime = fakeRuntime();
  const report = await runInstalledMainAcceptance(options(), { runtime });

  assert.deepEqual(report.persistent_contention, {
    action: "repair_required",
    attempt: 5,
    error_kind: "fulltext_publication_busy",
    kind: "fulltext",
    phase: "blocked",
    core_reason: "runtime_invariant",
    status: "expected_block_observed",
  });
  assert.equal(
    runtime.calls.filter(([operation]) => operation === "persistent-block")
      .length,
    1,
  );
  assert.ok(
    runtime.calls.some(
      (call) =>
        JSON.stringify(call) ===
        JSON.stringify(["clone", "fulltext-persistent-contention"]),
    ),
  );

  const incomplete = fakeRuntime();
  incomplete.waitForPersistentBlock = async () => ({
    action: "repair_required",
    attempt: 5,
    errorKind: "fulltext_publication_busy",
    phase: "blocked",
    repairReason: "artifact_unavailable",
  });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime: incomplete }),
    /persistent_contention_gate_failed/,
  );
});

test("requires a monotonic elapsed window that covers the reported backoff", () => {
  assert.equal(observedRealBackoff(1_000, 800), true);
  assert.equal(observedRealBackoff(1_000, 799), false);
  assert.equal(observedRealBackoff(1_000, -1), false);
  assert.equal(observedRealBackoff(499, 10_000), false);
});

test("redacts arbitrary tool failures and always performs cleanup", async () => {
  const runtime = fakeRuntime({ failAt: "cold" });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    (error) =>
      error instanceof AcceptanceError &&
      error.code === "acceptance_internal_failure" &&
      !error.message.includes("/private/path"),
  );
  assert.equal(runtime.calls.at(-1)[0], "cleanup");

  const cleanupRuntime = fakeRuntime({ cleanupFails: true });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime: cleanupRuntime }),
    (error) =>
      error instanceof AcceptanceError &&
      error.code === "cleanup_failed" &&
      !error.message.includes("/private/cleanup/path"),
  );
});
