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
import { verifyInstalledCombinedDiagnosticsExport } from "./diagnostics-export-acceptance.mjs";
import { executeRuntimeFaultAcceptance } from "./runtime-fault-acceptance-sequence.mjs";
import { REQUIRED_INSTALLED_VERSION } from "./source-bindings.mjs";

export function stableBindingsMatch(left, right) {
  return (
    left?.gitHead === right?.gitHead &&
    JSON.stringify(left?.source) === JSON.stringify(right?.source) &&
    left?.version === right?.version &&
    left?.dmgSha256 === right?.dmgSha256 &&
    left?.iconSha256 === right?.iconSha256 &&
    left?.composition?.composition_digest ===
      right?.composition?.composition_digest
  );
}

export function requireDeploymentBinding(deployment, verified) {
  if (
    !["install", "reinstall"].includes(
      deployment?.deploymentAction,
    ) ||
    !GIT_HEAD.test(deployment?.gitHead ?? "") ||
    !DIGEST.test(deployment?.dmgSha256 ?? "") ||
    !DIGEST.test(deployment?.compositionDigest ?? "") ||
    !DIGEST.test(verified?.iconSha256 ?? "") ||
    deployment?.gitHead !== verified?.gitHead ||
    JSON.stringify(deployment?.source) !==
      JSON.stringify(verified?.source) ||
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

function publicStaleControlEvidence(value) {
  if (
    !exactKeys(value, [
      "legacyContractReplaced",
      "newGenerationReady",
      "v29AuthorityPreserved",
    ]) ||
    Object.values(value).some((observed) => observed !== true)
  ) {
    fail("stale_control_evidence_invalid");
  }
  return Object.freeze({
    legacy_contract_replaced: true,
    new_generation_ready: true,
    v29_authority_preserved: true,
  });
}

function publicForeignControlEvidence(value) {
  if (
    !exactKeys(value, [
      "foreignEndpointPreserved",
      "newGenerationReady",
      "notAdopted",
      "notProbed",
      "v29AuthorityPreserved",
    ]) ||
    Object.values(value).some((observed) => observed !== true)
  ) {
    fail("foreign_control_evidence_invalid");
  }
  return Object.freeze({
    foreign_endpoint_preserved: true,
    new_generation_ready: true,
    not_adopted: true,
    not_probed: true,
    v29_authority_preserved: true,
  });
}

function publicSlowInitializationEvidence(value) {
  if (
    !exactKeys(value, [
      "sameInstance",
      "sameLaunch",
      "sameListener",
      "slowWindowObserved",
      "statusWithinTenSeconds",
      "v29AuthorityPreserved",
    ]) ||
    Object.values(value).some((observed) => observed !== true)
  ) {
    fail("slow_initialization_evidence_invalid");
  }
  return Object.freeze({
    same_instance: true,
    same_launch: true,
    same_listener: true,
    slow_window_observed: true,
    status_within_ten_seconds: true,
    v29_authority_preserved: true,
  });
}

function publicCombinedDiagnosticsEvidence(value) {
  if (
    !exactKeys(value, [
      "desktopContract",
      "nativeSaveDialog",
      "ownerOnlyFile",
      "boundedBytes",
      "daemonAvailableState",
      "daemonUnavailableState",
      "daemonDownLifecycleState",
    ]) ||
    value.desktopContract !== "resume-ir.desktop-diagnostics.v2" ||
    value.nativeSaveDialog !== true ||
    value.ownerOnlyFile !== true ||
    value.boundedBytes !== 256 * 1024 ||
    value.daemonAvailableState !== "included" ||
    value.daemonUnavailableState !== "unavailable" ||
    value.daemonDownLifecycleState !== "circuit_open"
  ) {
    fail("combined_diagnostics_evidence_invalid");
  }
  return Object.freeze({
    desktop_contract: value.desktopContract,
    daemon_available_export: "verified",
    daemon_unavailable_export: "verified",
    daemon_down_lifecycle_state: value.daemonDownLifecycleState,
    native_save_dialog: "automated",
    maximum_export_bytes: value.boundedBytes,
    file_permissions: "owner_only",
  });
}

export async function executeAcceptance(runtime, signal) {
  throwIfAborted(signal);
  await runtime.validatePreLockInputs();
  await runtime.requireInstalledFaultHarness();
  const sourceExpectation = await runtime.precheckSourceAuthority();
  await runtime.acquireAcceptanceLease();
  await runtime.bindSourceAuthorityAfterLease(sourceExpectation);
  await runtime.recoverInterruptedRuns();
  const deployment = await runtime.prepareInstalledRelease();
  const bindings = await runtime.verifyBindings();
  requireDeploymentBinding(deployment, bindings);
  const launchVerified = async (workspace) => {
    throwIfAborted(signal);
    await runtime.verifyBindings();
    await runtime.activateFaultCell(workspace);
    return runtime.launchApp(workspace);
  };
  const baseline = await runtime.createClone("cold-start");
  let session = await launchVerified(baseline);
  const cold = await runtime.waitForColdReady(session);
  if (cold?.v29AuthorityPreserved !== true) {
    fail("cold_v29_preservation_invalid");
  }
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
    core_reason: blocked.repairReason,
    status: "expected_block_observed",
  };

  const staleClone = await runtime.createClone("stale-control");
  const staleFixture = await runtime.prepareStaleControl(staleClone);
  const staleSession = await launchVerified(staleClone);
  const staleControl = publicStaleControlEvidence(
    await runtime.validateStaleControl(staleSession, staleFixture),
  );
  await runtime.quitApp(staleSession);
  await runtime.assertZeroResidue(staleSession);
  await runtime.closeControlFixture(staleFixture);

  const foreignClone = await runtime.createClone("foreign-control");
  const foreignFixture = await runtime.prepareForeignControl(foreignClone);
  const foreignSession = await launchVerified(foreignClone);
  const foreignControl = publicForeignControlEvidence(
    await runtime.validateForeignControl(foreignSession, foreignFixture),
  );
  await runtime.quitApp(foreignSession);
  await runtime.assertZeroResidue(foreignSession);
  await runtime.closeControlFixture(foreignFixture);

  const slowClone = await runtime.createClone("slow-initialization");
  const slowFault = await runtime.prepareFaultCell(
    slowClone,
    "slow_initialization",
  );
  const slowSession = await launchVerified(slowClone);
  const slowInitialization = publicSlowInitializationEvidence(
    await runtime.validateSlowInitialization(slowSession),
  );
  await runtime.quitApp(slowSession);
  await runtime.assertZeroResidue(slowSession);
  await runtime.releaseFaultCell(slowFault);

  const optionalRuntimeFaults = await executeRuntimeFaultAcceptance(
    runtime,
    launchVerified,
  );

  session = await launchVerified(baseline);
  const finalReady = await runtime.waitForReady(session);
  const finalEvidence = publicRecoveryEvidence(
    await runtime.validateRecoveryEvidence(session, finalReady, canary),
  );
  const diagnosticsEvidence = publicCombinedDiagnosticsEvidence(
    await verifyInstalledCombinedDiagnosticsExport(runtime, session, signal),
  );
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
      acceptance_lock: "held_for_entire_acceptance",
      lifecycle_lock: "owned_by_install_children",
      source_mutated: false,
      release_data_dir_override: false,
    },
    cold_start: {
      target_schema: 29,
      v29_data_preservation_and_index_recovery: "ready",
      v29_logical_authority_preserved: true,
      v29_manifest_identity_preserved: true,
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
    bootstrap_control: {
      stale: staleControl,
      foreign: foreignControl,
      slow_initialization: slowInitialization,
    },
    optional_runtime_faults: optionalRuntimeFaults,
    diagnostics: {
      daemon_contract: "resume-ir.diagnostics.v4",
      lifecycle_receipt: "validated",
      privacy_boundary: "redacted_local_aggregate",
      gui_combined_export: diagnosticsEvidence,
    },
  };
}
