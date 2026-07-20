import { spawn } from "node:child_process";
import { realpath } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";

import { ownerEvidencePath } from "../macos-owner-evidence-store.mjs";
import {
  LIFECYCLE_LOCK_FILE,
  acquireLifecycleLock,
  releaseLifecycleLock,
  requireLifecycleLockCapability,
} from "../macos-lifecycle-lock.mjs";
import { runBoundedTool } from "./bounded-process.mjs";
import {
  executeAcceptance,
  requireDeploymentBinding,
  stableBindingsMatch,
} from "./acceptance-sequence.mjs";
import {
  SYNTHETIC_SEARCH_REQUEST,
  SYNTHETIC_CANARY_TOKEN,
  validateInstalledReadyArtifacts,
  validateInstalledRecoveryEvidence,
} from "./acceptance-evidence.mjs";
import {
  COLD_READY_TIMEOUT_MS,
  CONTENTION_CONVERGENCE_TIMEOUT_MS,
  CONTENTION_ERROR_KINDS,
  CONTENTION_LOCKS,
  CONTENTION_TIMEOUT_MS,
  DATA_OWNER_LOCK,
  LIFECYCLE_RECEIPT,
  PERSISTENT_CONTENTION_TIMEOUT_MS,
  POLL_MS,
  READY_TIMEOUT_MS,
  AcceptanceError,
  asAcceptanceError,
  fail,
  throwIfAborted,
  wait,
} from "./core.mjs";
import {
  acquireExistingLock,
  createAcceptanceRunId,
  createCowCloneWorkspace,
  listRecoverableWorkspaces,
  pathsOverlap,
  readActiveStoreManifest,
  readPrivateJson,
  releaseExistingLock,
  requirePrivateFile,
  requireSecureDirectory,
  safeRemoveWorkspace,
  updateWorkspaceMarker,
} from "./filesystem-cow.mjs";
import {
  captureLifecycleReceiptBoundary,
  contentionStatus,
  persistentBlockedStatus,
  readDaemonConnection,
  readyStatus,
  requestJson,
  requestJsonPost,
  requestJsonPostAccepted,
  validateDaemonDiagnostics,
  validateLifecycleReceiptBoundary,
} from "./ipc-contracts.mjs";
import { normalizeAcceptanceOptions } from "./options.mjs";
import {
  assertNoInstalledRuntime,
  findOwnedDaemon,
  forceCleanupSession,
  groupProcesses,
  launchInstalledApp,
  pollStatus,
  quitInstalledApp,
  recordWorkspaceApplication,
  recoverStaleWorkspaceRuntime,
  strongKillOwnedDaemon,
  waitForNewGenerationReady,
} from "./process-lifecycle.mjs";
import { readProcessStartTime } from "./process-identity.mjs";
import {
  canaryImportCompleted,
  createSyntheticCanary,
  syntheticCanaryImportRequest,
  validateCanaryImportResponse,
  validateCanarySearchResponse,
} from "./synthetic-canary.mjs";
import { verifyInstalledSourceBindings } from "./source-bindings.mjs";
import {
  deployExactInstalledRelease,
  requireDefaultApplicationSupportRoot,
  sourceAuthorityMatches,
  verifyReleaseSourceBeforeDeployment,
} from "./release-deployment.mjs";

function receiptForbidden(options, session, connection) {
  return [
    options.authorizedSourceDataDir,
    session.dataDir,
    session.home,
    connection?.token,
    connection?.instanceId,
    SYNTHETIC_CANARY_TOKEN,
  ];
}

async function capturePersistedRecoveryBoundary(session, options, signal) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    try {
      const lifecycle = await readPrivateJson(
        path.join(session.dataDir, LIFECYCLE_RECEIPT),
        16 * 1024,
      );
      return captureLifecycleReceiptBoundary({
        capturedAtUnixMs: Date.now(),
        source: lifecycle.source,
        value: lifecycle.value,
        forbidden: receiptForbidden(options, session),
      });
    } catch (error) {
      if (
        !(error instanceof AcceptanceError) ||
        ![
          "lifecycle_receipt_boundary_invalid",
          "private_file_invalid",
          "private_json_invalid",
        ].includes(error.code)
      ) {
        throw error;
      }
    }
    await wait(POLL_MS);
  }
  fail("lifecycle_receipt_boundary_invalid");
}

async function validatePersistedRecoveryReceipt(
  session,
  options,
  boundary,
  signal,
) {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    try {
      const lifecycle = await readPrivateJson(
        path.join(session.dataDir, LIFECYCLE_RECEIPT),
        16 * 1024,
      );
      validateLifecycleReceiptBoundary({
        boundary,
        forbidden: receiptForbidden(options, session),
        source: lifecycle.source,
        value: lifecycle.value,
      });
      return;
    } catch (error) {
      if (
        !(error instanceof AcceptanceError) ||
        ![
          "lifecycle_receipt_boundary_invalid",
          "private_file_invalid",
          "private_json_invalid",
        ].includes(error.code)
      ) {
        throw error;
      }
    }
    await wait(POLL_MS);
  }
  fail("lifecycle_receipt_invalid");
}

async function validatePreLockInputs(options) {
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    fail("macos_arm64_required");
  }
  const [repo, source, temporary] = await Promise.all([
    requireSecureDirectory(options.repoRoot),
    requireSecureDirectory(options.authorizedSourceDataDir),
    requireSecureDirectory(options.temporaryParent),
  ]);
  if (
    pathsOverlap(source.resolved, repo.resolved) ||
    pathsOverlap(source.resolved, temporary.resolved) ||
    source.resolved === path.parse(source.resolved).root
  ) {
    fail("authorized_source_invalid");
  }
  return Object.freeze({
    repoRoot: repo.resolved,
    sourceDataDir: source.resolved,
    temporaryParent: temporary.resolved,
  });
}

export function observedRealBackoff(retryAfterMs, elapsedMs) {
  return (
    Number.isSafeInteger(retryAfterMs) &&
    retryAfterMs >= 500 &&
    Number.isFinite(elapsedMs) &&
    elapsedMs >= 0 &&
    elapsedMs + POLL_MS >= retryAfterMs
  );
}

async function verifySourceData(options) {
  const source = await requireSecureDirectory(options.authorizedSourceDataDir);
  const sourceManifest = await readActiveStoreManifest(source.resolved, {
    allowLegacyReadOnly: true,
  });
  if (sourceManifest.schema !== 28) fail("authorized_source_schema_invalid");
  await requirePrivateFile(path.join(source.resolved, DATA_OWNER_LOCK), {
    empty: true,
  });
  return sourceManifest.schema;
}

async function resolveRealApplicationSupportRoot() {
  let home;
  try {
    home = await realpath(os.userInfo().homedir);
  } catch {
    fail("owner_identity_unavailable");
  }
  const support = path.join(home, "Library", "Application Support");
  return (await requireSecureDirectory(support)).resolved;
}

export function createNativeAcceptanceRuntime(options, dependencies = {}) {
  const baseRunTool = dependencies.runTool ?? runBoundedTool;
  const signal = dependencies.signal;
  const runTool = (command, args, toolOptions = {}) =>
    baseRunTool(command, args, {
      ...toolOptions,
      signal: toolOptions.signal ?? signal,
    });
  const cleanupRunTool = (command, args, toolOptions = {}) =>
    baseRunTool(command, args, toolOptions);
  const spawnTool = dependencies.spawnTool ?? spawn;
  const now = dependencies.now ?? (() => performance.now());
  const runId = dependencies.runId ?? createAcceptanceRunId();
  const resolveApplicationSupportRoot =
    dependencies.resolveApplicationSupportRoot ??
    resolveRealApplicationSupportRoot;
  const requireDefaultSupportRoot =
    dependencies.requireDefaultApplicationSupportRoot ??
    requireDefaultApplicationSupportRoot;
  const acquireAcceptanceLock =
    dependencies.acquireLifecycleLock ?? acquireLifecycleLock;
  const releaseAcceptanceLock =
    dependencies.releaseLifecycleLock ?? releaseLifecycleLock;
  const requireAcceptanceLock =
    dependencies.requireLifecycleLockCapability ??
    requireLifecycleLockCapability;
  const validateInputs =
    dependencies.validatePreLockInputs ?? validatePreLockInputs;
  const verifyInstalledBindings =
    dependencies.verifyInstalledSourceBindings ?? verifyInstalledSourceBindings;
  const deployRelease =
    dependencies.deployExactInstalledRelease ?? deployExactInstalledRelease;
  const listStaleWorkspaces =
    dependencies.listRecoverableWorkspaces ?? listRecoverableWorkspaces;
  const recoverStaleRuntime =
    dependencies.recoverStaleWorkspaceRuntime ?? recoverStaleWorkspaceRuntime;
  const workspaces = [];
  const sessions = [];
  const locks = [];
  let applicationSupportRoot;
  let bindings;
  let cleanupPromise;
  let deployment;
  let sourceExpectation;
  let lifecycleLockCapability;
  let preflight;

  async function resolveSupportRoot() {
    if (!applicationSupportRoot) {
      const candidate = await resolveApplicationSupportRoot();
      applicationSupportRoot = await requireDefaultSupportRoot(candidate);
    }
    return applicationSupportRoot;
  }

  function requireLease() {
    if (!lifecycleLockCapability) fail("lifecycle_lock_required");
    try {
      requireAcceptanceLock(lifecycleLockCapability);
    } catch {
      fail("lifecycle_lock_lost");
    }
  }

  async function observeSourceAuthority() {
    if (!preflight) fail("preflight_required");
    return verifyReleaseSourceBeforeDeployment(
      { repoRoot: preflight.repoRoot },
      { ...dependencies.releaseDeploymentOverrides, runTool },
    );
  }

  async function requireMutationAuthority() {
    requireLease();
    throwIfAborted(signal);
    if (!sourceExpectation) fail("source_authority_required");
    const observed = await observeSourceAuthority();
    if (!sourceAuthorityMatches(sourceExpectation, observed)) {
      fail("source_authority_changed");
    }
    requireLease();
    throwIfAborted(signal);
    return observed;
  }

  async function releaseTrackedLock(capability) {
    await releaseExistingLock(capability);
    if (!capability.markerCleared) {
      await updateWorkspaceMarker(capability.workspace.root, runId, {
        helper: null,
      });
      capability.markerCleared = true;
    }
  }

  async function validateRepairDiagnostics(session, observed, expected) {
    const diagnostics = await requestJson(
      observed.connection.urls.diagnostics,
      observed.connection.token,
      undefined,
      signal,
    );
    validateDaemonDiagnostics(
      diagnostics,
      receiptForbidden(options, session, observed.connection),
      expected,
    );
  }

  const runtime = {
    async validatePreLockInputs() {
      throwIfAborted(signal);
      preflight = await validateInputs(options);
      return preflight;
    },
    async recoverInterruptedRuns() {
      if (!preflight) fail("preflight_required");
      requireLease();
      if (!sourceExpectation) fail("source_authority_required");
      throwIfAborted(signal);
      const stale = await listStaleWorkspaces(
        preflight.temporaryParent,
        runId,
      );
      for (const workspace of stale) {
        await requireMutationAuthority();
        await recoverStaleRuntime(workspace, cleanupRunTool);
        await requireMutationAuthority();
        await safeRemoveWorkspace(
          workspace.root,
          preflight.temporaryParent,
          workspace.runId,
        );
      }
    },
    async precheckSourceAuthority() {
      if (!preflight || lifecycleLockCapability || sourceExpectation) {
        fail("preflight_required");
      }
      throwIfAborted(signal);
      return observeSourceAuthority();
    },
    async bindSourceAuthorityAfterLease(expected) {
      requireLease();
      throwIfAborted(signal);
      const observed = await observeSourceAuthority();
      if (!sourceAuthorityMatches(expected, observed)) {
        fail("source_authority_changed");
      }
      sourceExpectation = Object.freeze({ ...observed });
      return sourceExpectation;
    },
    async prepareInstalledRelease() {
      requireLease();
      if (!sourceExpectation) fail("source_authority_required");
      await assertNoInstalledRuntime(runTool);
      deployment = await deployRelease(
        {
          applicationSupportRoot: await resolveSupportRoot(),
          repoRoot: preflight.repoRoot,
          signal,
          temporaryParent: preflight.temporaryParent,
          preverifiedSource: sourceExpectation,
        },
        {
          ...dependencies.releaseDeploymentOverrides,
          assertMutationAuthority: requireMutationAuthority,
          runTool,
        },
      );
      return deployment;
    },
    async acquireLifecycleLease() {
      if (!preflight || lifecycleLockCapability) fail("preflight_required");
      throwIfAborted(signal);
      lifecycleLockCapability = await acquireAcceptanceLock({
        lockFile: ownerEvidencePath(
          await resolveSupportRoot(),
          LIFECYCLE_LOCK_FILE,
        ),
      });
    },
    async verifyBindings() {
      requireLease();
      throwIfAborted(signal);
      const sourceSchema = await verifySourceData(options);
      const verified = await verifyInstalledBindings({
        applicationSupportRoot,
        repoRoot: preflight.repoRoot,
        runTool,
      });
      requireDeploymentBinding(deployment, verified);
      if (bindings && !stableBindingsMatch(bindings, verified)) {
        fail("installed_binding_drift");
      }
      bindings = Object.freeze({ ...verified, sourceSchema });
      return bindings;
    },
    async createClone() {
      await requireMutationAuthority();
      if (!bindings) fail("bindings_required");
      const workspace = await createCowCloneWorkspace({
        authorizedSourceDataDir: options.authorizedSourceDataDir,
        temporaryParent: options.temporaryParent,
        expectedComposition: bindings.composition,
        runId,
        runTool,
        acquireLock: (lockFile) => acquireExistingLock(lockFile, { spawnTool }),
        releaseLock: releaseExistingLock,
      });
      workspaces.push(workspace);
      return workspace;
    },
    async launchApp(workspace) {
      await requireMutationAuthority();
      if (!bindings) fail("bindings_required");
      await assertNoInstalledRuntime(runTool, bindings.executablePaths);
      const session = await launchInstalledApp(
        workspace,
        bindings.executablePaths,
        { runTool, spawnTool },
      );
      session.workspace = workspace;
      sessions.push(session);
      return session;
    },
    async waitForColdReady(session) {
      const ready = await pollStatus(
        session,
        readyStatus,
        COLD_READY_TIMEOUT_MS,
        null,
        signal,
      );
      const migrated = await readActiveStoreManifest(session.dataDir);
      if (migrated.schema !== 29) fail("cold_migration_invalid");
      await Promise.all(
        [
          DATA_OWNER_LOCK,
          "search-publication.lock",
          path.join("search-index", "snapshot-publication.lock"),
          path.join("vector-index", "snapshot-publication.lock"),
        ].map((relative) =>
          requirePrivateFile(path.join(session.dataDir, relative), {
            empty: true,
          }),
        ),
      );
      return ready;
    },
    async validateColdReadyArtifacts(session, observed) {
      if (!observed?.connection || !observed?.status) {
        fail("ready_evidence_invalid");
      }
      const diagnostics = await requestJson(
        observed.connection.urls.diagnostics,
        observed.connection.token,
        undefined,
        signal,
      );
      return validateInstalledReadyArtifacts({
        dataDir: session.dataDir,
        diagnostics,
        runTool,
        status: observed.status,
      });
    },
    async createSyntheticCanary(workspace) {
      await requireMutationAuthority();
      return createSyntheticCanary(workspace);
    },
    async importSyntheticCanary(session, canary, observed) {
      if (!observed?.connection || !observed?.status) {
        fail("synthetic_canary_import_invalid");
      }
      await requireMutationAuthority();
      const previousEpoch = observed.status.visible_epoch;
      validateCanaryImportResponse(
        await requestJsonPostAccepted(
          observed.connection.urls.imports,
          observed.connection.token,
          syntheticCanaryImportRequest(canary),
          { signal },
        ),
      );
      return pollStatus(
        session,
        (status) => canaryImportCompleted(status, previousEpoch),
        COLD_READY_TIMEOUT_MS,
        observed.instanceId,
        signal,
      );
    },
    waitForReady(session) {
      return pollStatus(
        session,
        readyStatus,
        READY_TIMEOUT_MS,
        null,
        signal,
      );
    },
    async validateRecoveryEvidence(session, observed, canary) {
      if (!observed?.connection || !observed?.status) {
        fail("ready_evidence_invalid");
      }
      const [diagnostics, search] = await Promise.all([
        requestJson(
          observed.connection.urls.diagnostics,
          observed.connection.token,
          undefined,
          signal,
        ),
        requestJsonPost(
          observed.connection.urls.search,
          observed.connection.token,
          SYNTHETIC_SEARCH_REQUEST,
          { signal },
        ),
      ]);
      validateCanarySearchResponse(search, observed.status.visible_epoch);
      return validateInstalledRecoveryEvidence({
        dataDir: session.dataDir,
        diagnostics,
        runTool,
        search,
        status: observed.status,
      });
    },
    findOwnedDaemon(session) {
      return findOwnedDaemon(session, runTool, signal);
    },
    async strongKillDaemon(session, target) {
      await requireMutationAuthority();
      return strongKillOwnedDaemon(session, target, runTool);
    },
    captureRecoveryBoundary(session) {
      return capturePersistedRecoveryBoundary(session, options, signal);
    },
    waitForNewGenerationReady(session, oldInstanceId) {
      return waitForNewGenerationReady(session, oldInstanceId, signal);
    },
    validateRecoveryBoundary(session, boundary) {
      return validatePersistedRecoveryReceipt(
        session,
        options,
        boundary,
        signal,
      );
    },
    async quitApp(session) {
      await requireMutationAuthority();
      await quitInstalledApp(session, runTool);
      await recordWorkspaceApplication(
        session.workspace,
        session,
        "app_stopped",
      );
    },
    async assertZeroResidue(session) {
      if ((await groupProcesses(session, runTool)).length !== 0) {
        fail("normal_quit_residue");
      }
      await assertNoInstalledRuntime(runTool, bindings.executablePaths);
    },
    async holdPublicationLock(workspace, kind) {
      await requireMutationAuthority();
      const parts = CONTENTION_LOCKS[kind];
      if (!parts) fail("contention_kind_invalid");
      const acquired = await acquireExistingLock(
        path.join(workspace.dataDir, ...parts),
        { spawnTool },
      );
      const capability = {
        ...acquired,
        kind,
        markerCleared: false,
        workspace,
      };
      locks.push(capability);
      try {
        const helperStartTime = await readProcessStartTime(
          capability.child.pid,
          runTool,
        );
        await updateWorkspaceMarker(workspace.root, runId, {
          helper: {
            kind: "publication_lock",
            pid: capability.child.pid,
            pgid: capability.child.pid,
            start_time: helperStartTime,
            executable: "/usr/bin/ruby",
            session_authority: runId,
            lock_kind: kind,
          },
        });
      } catch (error) {
        await releaseExistingLock(capability).catch(() => {});
        throw error;
      }
      return capability;
    },
    async waitForContention(session, kind) {
      const first = await pollStatus(
        session,
        (status) => contentionStatus(status, kind, 1),
        CONTENTION_TIMEOUT_MS,
        null,
        signal,
      );
      const firstObservedAt = now();
      const firstRetryAfter = first.status.repair_progress.retry_after_ms;
      if (!Number.isSafeInteger(firstRetryAfter) || firstRetryAfter < 500) {
        fail("contention_backoff_not_observed");
      }
      await validateRepairDiagnostics(session, first, {
        attempt: 1,
        kind,
        phase: "retry_wait",
      });
      const second = await pollStatus(
        session,
        (status) => contentionStatus(status, kind, 2),
        CONTENTION_TIMEOUT_MS,
        first.instanceId,
        signal,
      );
      const elapsed = now() - firstObservedAt;
      if (
        second.instanceId !== first.instanceId ||
        !observedRealBackoff(firstRetryAfter, elapsed)
      ) {
        fail("contention_backoff_not_observed");
      }
      await validateRepairDiagnostics(session, second, {
        attempt: 2,
        kind,
        phase: "retry_wait",
      });
      return {
        attempts: Object.freeze([1, 2]),
        backoffObservedMs: elapsed,
        errorKind: CONTENTION_ERROR_KINDS[kind],
        instanceId: second.instanceId,
      };
    },
    async releasePublicationLock(capability) {
      await requireMutationAuthority();
      return releaseTrackedLock(capability);
    },
    async waitForSameGenerationReady(session, instanceId) {
      await pollStatus(
        session,
        readyStatus,
        CONTENTION_CONVERGENCE_TIMEOUT_MS,
        instanceId,
        signal,
      );
    },
    async waitForPersistentBlock(session, kind) {
      const blocked = await pollStatus(
        session,
        (status) => persistentBlockedStatus(status, kind),
        PERSISTENT_CONTENTION_TIMEOUT_MS,
        null,
        signal,
      );
      await validateRepairDiagnostics(session, blocked, {
        kind,
        phase: "blocked",
      });
      return {
        action: blocked.status.error.action,
        attempt: blocked.status.repair_progress.attempt,
        instanceId: blocked.instanceId,
        phase: blocked.status.repair_progress.phase,
        errorKind: blocked.status.repair_progress.last_error_kind,
        repairReason: blocked.status.repair_reason,
      };
    },
    async validateDiagnostics(session) {
      throwIfAborted(signal);
      const connection = await readDaemonConnection(session.dataDir);
      if (connection.instanceId !== session.instanceId) {
        fail("daemon_restarted_unexpectedly");
      }
      const diagnostics = await requestJson(
        connection.urls.diagnostics,
        connection.token,
        undefined,
        signal,
      );
      validateDaemonDiagnostics(diagnostics, [
        options.authorizedSourceDataDir,
        session.dataDir,
        session.home,
        SYNTHETIC_CANARY_TOKEN,
        connection.token,
        connection.instanceId,
      ]);
    },
    cleanup() {
      if (cleanupPromise) return cleanupPromise;
      cleanupPromise = (async () => {
        let failed = false;
        for (const session of [...sessions].reverse()) {
          try {
            await forceCleanupSession(session, cleanupRunTool);
            await recordWorkspaceApplication(
              session.workspace,
              session,
              "app_stopped",
            );
          } catch {
            failed = true;
          }
        }
        for (const capability of [...locks].reverse()) {
          try {
            await releaseTrackedLock(capability);
          } catch {
            failed = true;
          }
        }
        let temporaryParent;
        try {
          temporaryParent = await realpath(options.temporaryParent);
        } catch {
          failed = true;
        }
        if (temporaryParent) {
          for (const workspace of [...workspaces].reverse()) {
            try {
              await safeRemoveWorkspace(
                workspace.root,
                temporaryParent,
                workspace.runId,
              );
            } catch {
              failed = true;
            }
          }
        }
        if (lifecycleLockCapability) {
          try {
            requireAcceptanceLock(lifecycleLockCapability);
          } catch {
            failed = true;
          }
          try {
            await releaseAcceptanceLock(lifecycleLockCapability);
          } catch {
            failed = true;
          }
        }
        if (failed) fail("cleanup_failed");
      })();
      return cleanupPromise;
    },
  };
  return runtime;
}

export async function runInstalledMainAcceptance(
  rawOptions,
  { runtime, signal } = {},
) {
  const options = normalizeAcceptanceOptions(rawOptions);
  const activeRuntime =
    runtime ?? createNativeAcceptanceRuntime(options, { signal });
  let result;
  let primaryError;
  try {
    result = await executeAcceptance(activeRuntime, signal);
  } catch (error) {
    primaryError = asAcceptanceError(error);
  }
  try {
    await activeRuntime.cleanup();
  } catch {
    fail("cleanup_failed");
  }
  if (primaryError) throw primaryError;
  return { ...result, cleanup: "temporary_clones_removed" };
}
