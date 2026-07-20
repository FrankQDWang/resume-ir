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
  runInstalledMainAcceptance,
} from "./orchestrator-receipt.mjs";
import { HEAD, diagnostics, fakeRuntime, options } from "./fixtures.mjs";

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
  assert.equal(report.outcome, "passed");
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
    report.data_boundary.lifecycle_lock,
    "held_for_entire_acceptance",
  );
  assert.equal(report.data_boundary.release_data_dir_override, false);
  assert.equal(report.supervised_strong_kill.targeting, "exact_owned_child");
  assert.equal(report.normal_quit_relaunch.process_residue, "none");
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
  assert.equal(report.persistent_contention.repair_reason, "runtime_invariant");
  assert.equal(
    report.diagnostics.gui_combined_export,
    "manual_required_native_save_dialog",
  );
  assert.equal(report.cleanup, "temporary_clones_removed");

  const clones = runtime.calls
    .filter(([operation]) => operation === "clone")
    .map(([, label]) => label);
  assert.deepEqual(clones, [
    "cold-start",
    "fulltext-contention",
    "vector-contention",
    "fulltext-persistent-contention",
  ]);
  assert.ok(
    runtime.calls.some(
      (call) =>
        JSON.stringify(call) === JSON.stringify(["strong-kill", 1, 47_111]),
    ),
  );
  assert.equal(runtime.calls.at(-1)[0], "cleanup");
  assert.deepEqual(runtime.calls.slice(0, 6), [
    ["preflight"],
    ["precheck-source-authority"],
    ["lifecycle-lock"],
    ["bind-source-authority-after-lease"],
    ["recover-interrupted"],
    ["prepare-release"],
  ]);
  for (let index = 0; index < runtime.calls.length; index += 1) {
    if (runtime.calls[index][0] === "launch") {
      assert.equal(runtime.calls[index - 1][0], "verify");
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
      ["precheck-source-authority"],
      ["lifecycle-lock"],
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
      ["verify"],
      ["launch", "cold-start", 5],
      ["ready", 5],
      ["recovery-evidence", 5, "ready-5"],
      ["diagnostics", 5],
      ["quit", 5],
      ["zero-residue", 5],
      ["verify"],
      ["cleanup"],
    ],
  );

  const publicBody = JSON.stringify(report);
  assert.equal(publicBody.includes("/synthetic/private/source"), false);
  assert.equal(publicBody.includes("raw stderr"), false);
  assert.equal(publicBody.includes("47111"), false);
});

test("source provenance is rechecked under the lifecycle lease before recovery", async () => {
  const runtime = fakeRuntime({ failAt: "source-recheck" });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /acceptance_internal_failure/,
  );
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["precheck-source-authority"],
    ["lifecycle-lock"],
    ["bind-source-authority-after-lease"],
    ["cleanup"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) => operation === "prepare-release"),
    false,
  );
});

test("read-only provenance fails before lifecycle acquisition or stale recovery", async () => {
  const runtime = fakeRuntime({ failAt: "source-provenance" });
  await assert.rejects(
    runInstalledMainAcceptance(options(), { runtime }),
    /acceptance_internal_failure/,
  );
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["precheck-source-authority"],
    ["cleanup"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) =>
      ["lifecycle-lock", "recover-interrupted", "prepare-release"].includes(
        operation,
      ),
    ),
    false,
  );
});

test("a concurrent run that cannot acquire the lifecycle lease never enters build", async () => {
  let rejectLease;
  const leaseBlocked = new Promise((_resolve, reject) => {
    rejectLease = reject;
  });
  const runtime = fakeRuntime();
  runtime.acquireLifecycleLease = async () => {
    runtime.calls.push(["lifecycle-lock"]);
    await leaseBlocked;
  };
  const running = runInstalledMainAcceptance(options(), { runtime });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["precheck-source-authority"],
    ["lifecycle-lock"],
  ]);
  assert.equal(
    runtime.calls.some(([operation]) => operation === "prepare-release"),
    false,
  );
  rejectLease(new Error("synthetic lifecycle contention"));
  await assert.rejects(running, /acceptance_internal_failure/);
  assert.deepEqual(runtime.calls, [
    ["preflight"],
    ["precheck-source-authority"],
    ["lifecycle-lock"],
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
    acquireLifecycleLock: async () => ({ synthetic: true }),
    requireDefaultApplicationSupportRoot: async (value) => value,
    requireLifecycleLockCapability: () => {
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
  await runtime.acquireLifecycleLease();
  await runtime.bindSourceAuthorityAfterLease(expected);

  const mutationAttempts = [
    () => runtime.recoverInterruptedRuns(),
    () => runtime.prepareInstalledRelease(),
    () => runtime.createClone("synthetic"),
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
  ];
  leaseValid = false;
  for (const mutate of mutationAttempts) {
    await assert.rejects(mutate(), /lifecycle_lock_lost/);
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
    repair_reason: "runtime_invariant",
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
