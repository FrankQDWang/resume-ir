import {
  ACCEPTANCE_SCHEMA,
  CONTENTION_ERROR_KINDS,
  DIGEST,
  GIT_HEAD,
  TARGET_TRIPLE,
  exactKeys,
  fail,
  throwIfAborted,
} from "./core.mjs";
import { REQUIRED_INSTALLED_VERSION } from "./source-bindings.mjs";

export function stableBindingsMatch(left, right) {
  return (
    left?.gitHead === right?.gitHead &&
    left?.version === right?.version &&
    left?.dmgSha256 === right?.dmgSha256 &&
    left?.iconSha256 === right?.iconSha256 &&
    left?.composition?.composition_digest ===
      right?.composition?.composition_digest
  );
}

export function requireDeploymentBinding(deployment, verified) {
  if (
    !["install", "upgrade", "reinstall"].includes(
      deployment?.deploymentAction,
    ) ||
    !GIT_HEAD.test(deployment?.gitHead ?? "") ||
    !DIGEST.test(deployment?.dmgSha256 ?? "") ||
    !DIGEST.test(deployment?.compositionDigest ?? "") ||
    !DIGEST.test(verified?.iconSha256 ?? "") ||
    deployment?.gitHead !== verified?.gitHead ||
    deployment?.version !== REQUIRED_INSTALLED_VERSION ||
    verified?.version !== REQUIRED_INSTALLED_VERSION ||
    deployment?.dmgSha256 !== verified?.dmgSha256 ||
    deployment?.compositionDigest !==
      verified?.composition?.composition_digest
  ) {
    fail("installed_deployment_binding_mismatch");
  }
}

function publicRecoveryEvidence(value) {
  if (
    !exactKeys(value, [
      "generationAgreement",
      "metadataArtifactBound",
      "searchWitness",
    ]) ||
    value.generationAgreement !== true ||
    value.metadataArtifactBound !== true ||
    value.searchWitness !== true
  ) {
    fail("ready_evidence_invalid");
  }
  return Object.freeze({
    generation_agreement: true,
    metadata_artifact_bound: true,
    search_witness: true,
  });
}

function requireReadyArtifactEvidence(value) {
  if (
    !exactKeys(value, ["generationAgreement", "metadataArtifactBound"]) ||
    value.generationAgreement !== true ||
    value.metadataArtifactBound !== true
  ) {
    fail("ready_evidence_invalid");
  }
}

export async function executeAcceptance(runtime, signal) {
  throwIfAborted(signal);
  await runtime.validatePreLockInputs();
  const sourceExpectation = await runtime.precheckSourceAuthority();
  await runtime.acquireLifecycleLease();
  await runtime.bindSourceAuthorityAfterLease(sourceExpectation);
  await runtime.recoverInterruptedRuns();
  const deployment = await runtime.prepareInstalledRelease();
  const bindings = await runtime.verifyBindings();
  requireDeploymentBinding(deployment, bindings);
  const launchVerified = async (workspace) => {
    throwIfAborted(signal);
    await runtime.verifyBindings();
    return runtime.launchApp(workspace);
  };
  const baseline = await runtime.createClone("cold-start");
  let session = await launchVerified(baseline);
  const cold = await runtime.waitForColdReady(session);
  requireReadyArtifactEvidence(
    await runtime.validateColdReadyArtifacts(session, cold),
  );
  const canary = await runtime.createSyntheticCanary(baseline);
  const canaryReady = await runtime.importSyntheticCanary(
    session,
    canary,
    cold,
  );
  const coldEvidence = publicRecoveryEvidence(
    await runtime.validateRecoveryEvidence(session, canaryReady, canary),
  );
  const daemon = await runtime.findOwnedDaemon(session);
  const recoveryBoundary = await runtime.captureRecoveryBoundary(session);
  await runtime.strongKillDaemon(session, daemon);
  await runtime.waitForNewGenerationReady(session, cold.instanceId);
  await runtime.validateRecoveryBoundary(session, recoveryBoundary);
  const recoveredReady = await runtime.waitForReady(session);
  const recoveredEvidence = publicRecoveryEvidence(
    await runtime.validateRecoveryEvidence(session, recoveredReady, canary),
  );
  await runtime.quitApp(session);
  await runtime.assertZeroResidue(session);

  const contention = {};
  for (const kind of ["fulltext", "vector"]) {
    const clone = await runtime.createClone(`${kind}-contention`);
    const lock = await runtime.holdPublicationLock(clone, kind);
    const contender = await launchVerified(clone);
    const observed = await runtime.waitForContention(contender, kind);
    await runtime.releasePublicationLock(lock);
    await runtime.waitForSameGenerationReady(contender, observed.instanceId);
    await runtime.quitApp(contender);
    await runtime.assertZeroResidue(contender);
    contention[kind] = {
      attempts_observed: observed.attempts,
      backoff_window_observed: observed.backoffObservedMs > 0,
      bounded_retry: true,
      convergence: "ready",
      daemon_restart: false,
      error_kind: observed.errorKind,
    };
  }

  const persistentKind = "fulltext";
  const persistentClone = await runtime.createClone(
    `${persistentKind}-persistent-contention`,
  );
  const persistentLock = await runtime.holdPublicationLock(
    persistentClone,
    persistentKind,
  );
  const persistentContender = await launchVerified(persistentClone);
  const blocked = await runtime.waitForPersistentBlock(
    persistentContender,
    persistentKind,
  );
  if (
    blocked.attempt !== 5 ||
    blocked.phase !== "blocked" ||
    blocked.action !== "repair_required" ||
    blocked.errorKind !== CONTENTION_ERROR_KINDS.fulltext ||
    blocked.repairReason !== "runtime_invariant"
  ) {
    fail("persistent_contention_gate_failed");
  }
  await runtime.quitApp(persistentContender);
  await runtime.assertZeroResidue(persistentContender);
  await runtime.releasePublicationLock(persistentLock);
  const persistent = {
    action: blocked.action,
    attempt: blocked.attempt,
    error_kind: blocked.errorKind,
    kind: persistentKind,
    phase: blocked.phase,
    repair_reason: blocked.repairReason,
    status: "expected_block_observed",
  };

  session = await launchVerified(baseline);
  const finalReady = await runtime.waitForReady(session);
  const finalEvidence = publicRecoveryEvidence(
    await runtime.validateRecoveryEvidence(session, finalReady, canary),
  );
  await runtime.validateDiagnostics(session);
  await runtime.quitApp(session);
  await runtime.assertZeroResidue(session);

  const finalBindings = await runtime.verifyBindings();
  if (!stableBindingsMatch(finalBindings, bindings)) {
    fail("installed_binding_drift");
  }

  return {
    schema_version: ACCEPTANCE_SCHEMA,
    outcome: "passed",
    bindings: {
      git_head: bindings.gitHead,
      version: bindings.version,
      target_triple: TARGET_TRIPLE,
      composition_digest: bindings.composition.composition_digest,
      icon_sha256: bindings.iconSha256,
      installed_location: "system_applications",
    },
    deployment: {
      action: deployment.deploymentAction,
      built_dmg_verified: true,
      installed_version: deployment.version,
      source: "clean_origin_main",
    },
    data_boundary: {
      source_authorization: "explicit_cli_argument",
      source_schema: bindings.sourceSchema,
      clone: "apfs_copy_on_write",
      lifecycle_lock: "held_for_entire_acceptance",
      source_mutated: false,
      release_data_dir_override: false,
    },
    cold_start: {
      target_schema: 29,
      migration_and_index_recovery: "ready",
      index_health: "ready",
      recovery_evidence: coldEvidence,
    },
    supervised_strong_kill: {
      lifecycle_receipt_boundary: "current_child_exit_to_next_generation",
      targeting: "exact_owned_child",
      new_generation: "ready",
      recovery_evidence: recoveredEvidence,
    },
    normal_quit_relaunch: {
      termination: "targeted_ns_running_application",
      relaunch: "ready",
      process_residue: "none",
      recovery_evidence: finalEvidence,
    },
    contention,
    persistent_contention: persistent,
    diagnostics: {
      daemon_contract: "resume-ir.diagnostics.v3",
      lifecycle_receipt: "validated",
      privacy_boundary: "redacted_local_aggregate",
      gui_combined_export: "manual_required_native_save_dialog",
    },
  };
}
