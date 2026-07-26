import { readdir, unlink } from "node:fs/promises";
import { userInfo } from "node:os";
import path from "node:path";

import {
  DAEMON_EXECUTABLE,
  DEFAULT_INSTALLED_EXECUTABLES,
  EMBEDDING_EXECUTABLE,
  INSTALLED_APP_BUNDLE,
  PDF_RENDER_EXECUTABLE,
  currentUid,
  fail,
  validAbsolutePath,
  wait,
} from "./core.mjs";
import {
  BACKUP_SUFFIX,
  atomicRename,
  backupDigest,
  backupPath,
  fileSha256,
  missingObject,
  optionalMetadata,
  permissionFailure,
  requireSafeFile,
  sameIdentity,
  systemAtomicRename,
} from "./native-fault-file-ops.mjs";
import {
  trustedRecoveryDigest,
  trustedRecoveryTarget,
} from "./native-fault-recovery-authority.mjs";
import {
  SLOW_MONITOR_TIMEOUT_MS,
  SLOW_RUNTIME_STOP_MS,
  createSlowInitializationController,
  defaultProcessTable,
} from "./native-slow-initialization.mjs";
import { readProcessStartTime } from "./process-identity.mjs";
import {
  REQUIRED_FAULT_CELLS,
  runtimeFaultCase,
} from "./native-runtime-fault-plan.mjs";
import {
  INVALID_FAULT_SHA256,
  runtimeFaultTargets,
  systemDenyExecution,
  targetForMutation,
  writeInvalidReplacement,
  writeStartFailureReplacement,
} from "./native-runtime-fault-mutations.mjs";

export { REQUIRED_FAULT_CELLS } from "./native-runtime-fault-plan.mjs";

export { SLOW_RUNTIME_STOP_MS };

function requireExactExecutables(executablePaths, expected) {
  if (
    executablePaths?.desktop !== expected.desktop ||
    executablePaths?.daemon !== expected.daemon ||
    executablePaths?.embedding_runtime !== expected.embedding_runtime ||
    executablePaths?.pdf_renderer !== expected.pdf_renderer
  ) {
    fail("installed_fault_target_invalid");
  }
}


function createHarness({
  appBundle,
  cleanupRunTool,
  delay,
  denyExecution,
  expectedExecutables,
  killProcess,
  listProcesses,
  loadRecoveryAuthority,
  monitorTimeoutMs,
  now,
  readStartTime,
  renameForActivation,
  renameForCleanup,
  recoveryDigest,
  runTool,
  slowStopMs,
  uid,
}) {
  const targets = runtimeFaultTargets(appBundle, expectedExecutables);
  const records = new WeakMap();
  const slowInitialization = createSlowInitializationController({
    cleanupRunTool,
    delay,
    embeddingExecutable: targets.embedding,
    expectedExecutables,
    killProcess,
    listProcesses,
    monitorTimeoutMs,
    now,
    readStartTime,
    runTool,
    slowStopMs,
  });

  async function removeFaultReplacement(mutation) {
    const target = await requireSafeFile(mutation.target, mutation);
    const digest = await fileSha256(mutation.target);
    if (
      digest !== INVALID_FAULT_SHA256 &&
      (digest !== mutation.digest || sameIdentity(target, mutation.identity))
    ) {
      fail("installed_fault_restore_failed");
    }
    try {
      await unlink(mutation.target);
    } catch (error) {
      if (permissionFailure(error)) fail("installed_fault_permission_denied");
      fail("installed_fault_restore_failed");
    }
  }

  async function restoreMutation(mutation) {
    const targetMetadata = await optionalMetadata(mutation.target);
    const backupMetadata = await optionalMetadata(mutation.backup);
    if (
      targetMetadata &&
      backupMetadata &&
      mutation.activation === "missing"
    ) {
      fail("installed_fault_backup_conflict");
    }
    if (!targetMetadata && !backupMetadata) fail("installed_fault_restore_failed");
    if (!targetMetadata) {
      const backupIdentity = await requireSafeFile(mutation.backup, mutation);
      if (
        !sameIdentity(backupIdentity, mutation.identity) ||
        (await fileSha256(mutation.backup)) !== mutation.digest
      ) {
        fail("installed_fault_backup_changed");
      }
      await renameForCleanup(
        mutation.backup,
        mutation.target,
        "installed_fault_restore_failed",
      );
    } else if (mutation.activation !== "missing") {
      await removeFaultReplacement(mutation);
      await renameForCleanup(
        mutation.backup,
        mutation.target,
        "installed_fault_restore_failed",
      );
    }
    const restored = await requireSafeFile(mutation.target, mutation);
    if (
      !sameIdentity(restored, mutation.identity) ||
      (await fileSha256(mutation.target)) !== mutation.digest ||
      (await optionalMetadata(mutation.backup))
    ) {
      fail("installed_fault_restore_failed");
    }
  }

  async function recoverTarget(
    target,
    executable,
    backupRecord,
    recoveryAuthority,
  ) {
    const backup = backupRecord?.file;
    const [targetMetadata, backupMetadata] = await Promise.all([
      optionalMetadata(target),
      backup ? optionalMetadata(backup) : null,
    ]);
    if (!backupMetadata) return;
    const identity = await requireSafeFile(backup, { executable, uid });
    if ((await fileSha256(backup)) !== backupRecord.digest) {
      fail("installed_fault_backup_changed");
    }
    if (targetMetadata) {
      const replacement = await requireSafeFile(target, { executable, uid });
      const targetDigest = await fileSha256(target);
      if (
        targetDigest !== INVALID_FAULT_SHA256 &&
        (targetDigest !== backupRecord.digest || sameIdentity(replacement, identity))
      ) {
        fail("installed_fault_backup_conflict");
      }
      try {
        await unlink(target);
      } catch (error) {
        if (permissionFailure(error)) fail("installed_fault_permission_denied");
        fail("installed_fault_recovery_failed");
      }
    }
    let trusted;
    try {
      trusted = trustedRecoveryTarget(recoveryAuthority, appBundle, target);
      if ((await recoveryDigest(backup, trusted.digest)) !== trusted.sha256) {
        fail("installed_fault_backup_untrusted");
      }
    } catch (error) {
      if (error?.code === "installed_fault_backup_untrusted") throw error;
      fail("installed_fault_recovery_authority_invalid");
    }
    await renameForActivation(
      backup,
      target,
      "installed_fault_restore_failed",
    );
    const restored = await requireSafeFile(target, { executable, uid });
    if (
      !sameIdentity(identity, restored) ||
      (await fileSha256(target)) !== backupRecord.digest ||
      (await optionalMetadata(backup))
    ) {
      fail("installed_fault_restore_failed");
    }
  }

  async function discoverBackups() {
    const targetFiles = [
      targets.embedding,
      targets.ocrEngine,
      targets.pdfRenderer,
      targets.classifierModel,
    ];
    const parents = new Set(targetFiles.map((file) => path.dirname(file)));
    const discovered = new Map();
    for (const parent of parents) {
      let entries;
      try {
        entries = await readdir(parent);
      } catch (error) {
        if (missingObject(error)) continue;
        if (permissionFailure(error)) fail("installed_fault_permission_denied");
        fail("installed_fault_recovery_failed");
      }
      for (const entry of entries.filter((name) => name.endsWith(BACKUP_SUFFIX))) {
        const matches = targetFiles
          .filter((target) => path.dirname(target) === parent)
          .map((target) => ({ target, digest: backupDigest(target, entry) }))
          .filter(({ digest }) => digest !== null);
        if (matches.length !== 1 || discovered.has(matches[0].target)) {
          fail("installed_fault_backup_unknown");
        }
        discovered.set(matches[0].target, {
          digest: matches[0].digest,
          file: path.join(parent, entry),
        });
      }
    }
    return discovered;
  }

  return Object.freeze({
    supportedCells: REQUIRED_FAULT_CELLS,
    async recover() {
      const backups = await discoverBackups();
      if (backups.size === 0) return;
      let recoveryAuthority;
      try {
        recoveryAuthority = await loadRecoveryAuthority();
      } catch {
        fail("installed_fault_recovery_authority_invalid");
      }
      await recoverTarget(
        targets.embedding,
        true,
        backups.get(targets.embedding),
        recoveryAuthority,
      );
      await recoverTarget(
        targets.pdfRenderer,
        true,
        backups.get(targets.pdfRenderer),
        recoveryAuthority,
      );
      await recoverTarget(
        targets.ocrEngine,
        true,
        backups.get(targets.ocrEngine),
        recoveryAuthority,
      );
      await recoverTarget(
        targets.classifierModel,
        false,
        backups.get(targets.classifierModel),
        recoveryAuthority,
      );
    },
    async prepare({ cell, dataDir, executablePaths }) {
      if (
        !REQUIRED_FAULT_CELLS.includes(cell) ||
        !validAbsolutePath(dataDir)
      ) {
        fail("installed_fault_cell_invalid");
      }
      requireExactExecutables(executablePaths, expectedExecutables);
      const token = Object.freeze({});
      const definition =
        cell === "slow_initialization" ? null : runtimeFaultCase(cell);
      records.set(token, {
        activated: false,
        cell,
        definition,
        mutations:
          definition?.mutations.map((mutation) => ({
            ...targetForMutation(mutation, targets),
            backup: null,
            backedUp: false,
            digest: null,
            identity: null,
            replacementWritten: false,
            uid,
          })) ?? [],
        restored: false,
        slow:
          cell === "slow_initialization"
            ? slowInitialization.prepare(dataDir)
            : null,
      });
      return token;
    },
    async activate(token) {
      const record = records.get(token);
      if (!record || record.activated || record.restored) {
        fail("installed_fault_activation_invalid");
      }
      await slowInitialization.requireNoInstalledRuntime();
      if (record.cell === "slow_initialization") {
        record.activated = true;
        slowInitialization.activate(record.slow);
        return;
      }
      for (const mutation of record.mutations) {
        mutation.identity = await requireSafeFile(mutation.target, mutation);
        try {
          mutation.digest = await fileSha256(mutation.target);
        } catch (error) {
          if (permissionFailure(error)) fail("installed_fault_permission_denied");
          fail("installed_fault_target_invalid");
        }
        mutation.backup = backupPath(mutation.target, mutation.digest);
        if (await optionalMetadata(mutation.backup)) {
          fail("installed_fault_backup_conflict");
        }
        await renameForActivation(
          mutation.target,
          mutation.backup,
          "installed_fault_activation_failed",
        );
        mutation.backedUp = true;
        const moved = await requireSafeFile(mutation.backup, mutation);
        if (
          !sameIdentity(moved, mutation.identity) ||
          (await fileSha256(mutation.backup)) !== mutation.digest ||
          (await optionalMetadata(mutation.target))
        ) {
          fail("installed_fault_activation_failed");
        }
        if (mutation.activation === "invalid") {
          await writeInvalidReplacement(mutation);
          mutation.replacementWritten = true;
        } else if (mutation.activation === "deny_execution_after_attestation") {
          await writeStartFailureReplacement(mutation, denyExecution);
          mutation.replacementWritten = true;
        } else if (mutation.activation !== "missing") {
          fail("installed_fault_cell_invalid");
        }
      }
      record.activated = true;
    },
    async restore(token, { requireCompleted = false } = {}) {
      const record = records.get(token);
      if (!record) fail("installed_fault_handle_invalid");
      if (record.restored) {
        if (
          requireCompleted &&
          record.cell === "slow_initialization"
        ) {
          slowInitialization.requireCompleted(record.slow, true);
        }
        return;
      }
      if (record.cell === "slow_initialization") {
        await slowInitialization.restore(record.slow, requireCompleted);
        record.restored = true;
        return;
      }
      for (const mutation of [...record.mutations].reverse()) {
        if (mutation.backedUp) await restoreMutation(mutation);
      }
      record.restored = true;
      if (requireCompleted && !record.activated) {
        fail("installed_fault_not_activated");
      }
    },
  });
}

export function createInstalledNativeFaultHarness({
  cleanupRunTool,
  loadRecoveryAuthority,
  runTool,
} = {}) {
  if (
    typeof runTool !== "function" ||
    typeof cleanupRunTool !== "function" ||
    typeof loadRecoveryAuthority !== "function"
  ) {
    fail("installed_fault_harness_unavailable");
  }
  return createHarness({
    appBundle: INSTALLED_APP_BUNDLE,
    cleanupRunTool,
    delay: wait,
    denyExecution: (file) =>
      systemDenyExecution(runTool, userInfo().username, file),
    expectedExecutables: DEFAULT_INSTALLED_EXECUTABLES,
    killProcess: process.kill.bind(process),
    listProcesses: defaultProcessTable,
    loadRecoveryAuthority,
    monitorTimeoutMs: SLOW_MONITOR_TIMEOUT_MS,
    now: Date.now,
    readStartTime: readProcessStartTime,
    renameForActivation: (source, destination, code) =>
      systemAtomicRename(runTool, source, destination, code),
    renameForCleanup: (source, destination, code) =>
      systemAtomicRename(cleanupRunTool, source, destination, code),
    recoveryDigest: trustedRecoveryDigest,
    runTool,
    slowStopMs: SLOW_RUNTIME_STOP_MS,
    uid: currentUid(),
  });
}

export function createNativeFaultHarnessForTesting({
  appBundle,
  cleanupRunTool = async () => {},
  delay = wait,
  denyExecution = async () => {},
  expectedExecutables = {
    desktop: path.join(appBundle, "Contents", "MacOS", "resume-desktop"),
    daemon: path.join(appBundle, "Contents", "MacOS", "resume-daemon"),
    embedding_runtime: path.join(
      appBundle,
      "Contents",
      "MacOS",
      "resume-embedding-runtime",
    ),
    pdf_renderer: path.join(
      appBundle,
      "Contents",
      "MacOS",
      "resume-pdf-render-runtime",
    ),
  },
  killProcess = () => {},
  listProcesses = async () => [],
  loadRecoveryAuthority = async () => {
    fail("installed_fault_recovery_authority_invalid");
  },
  monitorTimeoutMs = SLOW_MONITOR_TIMEOUT_MS,
  now = Date.now,
  readStartTime = async () => "Mon Jan  1 00:00:00 2024",
  renameForActivation = atomicRename,
  renameForCleanup = atomicRename,
  recoveryDigest = (file) => fileSha256(file),
  runTool = async () => {},
  slowStopMs = SLOW_RUNTIME_STOP_MS,
  uid = currentUid(),
}) {
  return createHarness({
    appBundle,
    cleanupRunTool,
    delay,
    denyExecution,
    expectedExecutables,
    killProcess,
    listProcesses,
    loadRecoveryAuthority,
    monitorTimeoutMs,
    now,
    readStartTime,
    renameForActivation,
    renameForCleanup,
    recoveryDigest,
    runTool,
    slowStopMs,
    uid,
  });
}

export const NATIVE_FAULT_TARGETS = Object.freeze({
  daemon: DAEMON_EXECUTABLE,
  ...runtimeFaultTargets(INSTALLED_APP_BUNDLE, {
    embedding_runtime: EMBEDDING_EXECUTABLE,
    pdf_renderer: PDF_RENDER_EXECUTABLE,
  }),
});
