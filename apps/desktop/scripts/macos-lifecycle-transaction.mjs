import { lstat, open, rename, rm } from "node:fs/promises";

import {
  advanceLifecycleJournal,
  persistLifecycleJournal,
  removeLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";
import { requireLifecycleLockCapability } from "./macos-lifecycle-lock.mjs";
import { lifecycleWorkspacePaths } from "./macos-lifecycle-workspace.mjs";

function transactionError(message) {
  return new Error(message);
}

async function pathExists(target) {
  try {
    await lstat(target);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw transactionError("lifecycle workspace is unavailable");
  }
}

function sameReceipt(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}

async function readOptionalReceipt({ readReceipt, applicationSupportRoot }) {
  return readReceipt({ applicationSupportRoot, allowMissing: true });
}

async function defaultSyncDirectory(directory) {
  const handle = await open(directory, "r");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

function controller({
  journal,
  applicationSupportRoot,
  persistJournal,
  removeJournal,
}) {
  let current = journal;
  return {
    get journal() {
      return current;
    },
    async phase(phase) {
      const next = advanceLifecycleJournal({ journal: current, phase });
      await persistJournal({ applicationSupportRoot, journal: next });
      current = next;
      return next;
    },
    async finish(phase) {
      await this.phase(phase);
      await removeJournal({
        applicationSupportRoot,
        expectedJournal: current,
      });
    },
  };
}

async function moveDurably({
  source,
  target,
  move,
  directory,
  syncDirectory,
  message = "lifecycle rename failed",
}) {
  try {
    await move(source, target);
    await syncDirectory(directory);
  } catch {
    throw transactionError(message);
  }
}

async function removeTombstoneDurably({
  target,
  remove,
  directory,
  syncDirectory,
}) {
  await remove(target, { recursive: true, force: true });
  await syncDirectory(directory);
}

function transactionDependencies(options) {
  return {
    move: options.filesystem?.rename ?? rename,
    remove: options.filesystem?.rm ?? rm,
    syncDirectory:
      options.filesystem?.syncDirectory ?? defaultSyncDirectory,
    persistJournal: options.persistJournal ?? persistLifecycleJournal,
    removeJournal: options.removeJournal ?? removeLifecycleJournal,
  };
}

function workspace(options) {
  const derived = lifecycleWorkspacePaths({
    applicationsRoot: options.applicationsRoot,
    operation: options.journal.operation,
    transactionId: options.journal.transaction_id,
  });
  return {
    partial: options.partial ?? derived.partial,
    stage:
      options.stage ??
      (options.journal.operation === "uninstall"
        ? derived.quarantine
        : derived.ready),
    backup: options.backup ?? derived.backup,
    tombstones: {
      stage: options.tombstones?.stage ?? derived.stageTombstone,
      target: options.tombstones?.target ?? derived.targetTombstone,
      backup: options.tombstones?.backup ?? derived.backupTombstone,
    },
  };
}

async function quarantineAndGc({
  source,
  tombstone,
  verify,
  beforePhase,
  tombstonedPhase,
  options,
  tx,
  dependencies,
}) {
  const [sourcePresent, tombstonePresent] = await Promise.all([
    pathExists(source),
    pathExists(tombstone),
  ]);
  if (sourcePresent && tombstonePresent) {
    throw transactionError("lifecycle tombstone state is ambiguous");
  }
  if (sourcePresent) {
    if (verify) await verify(source, options.journal);
    await tx.phase(beforePhase);
    if (verify) await verify(source, options.journal);
    await moveDurably({
      source,
      target: tombstone,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "lifecycle tombstone rename failed",
    });
    await tx.phase(tombstonedPhase);
  } else if (!tombstonePresent) {
    return false;
  } else if (tx.journal.phase === beforePhase) {
    await tx.phase(tombstonedPhase);
  } else if (tx.journal.phase !== tombstonedPhase) {
    throw transactionError("lifecycle tombstone phase is invalid");
  }
  await removeTombstoneDurably({
    target: tombstone,
    remove: dependencies.remove,
    directory: options.applicationsRoot,
    syncDirectory: dependencies.syncDirectory,
  });
  return true;
}

function tombstoneContracts(operation) {
  if (operation === "install") {
    return {
      stage: ["install_before_stage_cleanup", "install_stage_tombstoned"],
      target: ["install_before_target_cleanup", "install_target_tombstoned"],
    };
  }
  if (operation === "reinstall") {
    const prefix = "reinstall";
    return {
      stage: [
        `${prefix}_before_stage_cleanup`,
        `${prefix}_stage_tombstoned`,
      ],
      target: [
        `${prefix}_before_recovery_target_cleanup`,
        `${prefix}_target_tombstoned`,
      ],
      backup: [
        `${prefix}_before_backup_cleanup`,
        `${prefix}_backup_tombstoned`,
      ],
    };
  }
  return {
    stage: [
      "uninstall_before_quarantine_cleanup",
      "uninstall_quarantine_tombstoned",
    ],
  };
}

async function drainExistingTombstone(options, tx, dependencies, paths) {
  const contracts = tombstoneContracts(options.journal.operation);
  const present = [];
  for (const role of Object.keys(contracts)) {
    if (await pathExists(paths.tombstones[role])) present.push(role);
  }
  if (present.length > 1) {
    throw transactionError("lifecycle tombstone state is ambiguous");
  }
  if (present.length === 0) return undefined;
  const role = present[0];
  const [beforePhase, tombstonedPhase] = contracts[role];
  if (![beforePhase, tombstonedPhase].includes(tx.journal.phase)) {
    throw transactionError("lifecycle tombstone phase is invalid");
  }
  if (tx.journal.phase === beforePhase) await tx.phase(tombstonedPhase);
  await removeTombstoneDurably({
    target: paths.tombstones[role],
    remove: dependencies.remove,
    directory: options.applicationsRoot,
    syncDirectory: dependencies.syncDirectory,
  });
  return role;
}

async function cleanupPartialStage(options, tx, dependencies, paths) {
  if (!(await pathExists(paths.partial))) return false;
  if (await pathExists(paths.stage)) {
    throw transactionError("lifecycle stage state is ambiguous");
  }
  const allowed = new Set([
    `${options.journal.operation}_prepared`,
    `${options.journal.operation}_before_stage_publish`,
  ]);
  if (!allowed.has(tx.journal.phase)) {
    throw transactionError("lifecycle partial stage phase is invalid");
  }
  const prefix = options.journal.operation;
  await quarantineAndGc({
    source: paths.partial,
    tombstone: paths.tombstones.stage,
    beforePhase: `${prefix}_before_stage_cleanup`,
    tombstonedPhase: `${prefix}_stage_tombstoned`,
    options,
    tx,
    dependencies,
  });
  return true;
}

async function requireExpectedReceipt(options, expected, allowMissing = false) {
  const receipt = await readOptionalReceipt(options);
  if (receipt === undefined && allowMissing) return undefined;
  if (!sameReceipt(receipt, expected)) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  return receipt;
}

async function persistExpectedReceipt(options, expected) {
  await options.persistReceipt({
    applicationSupportRoot: options.applicationSupportRoot,
    receipt: expected,
  });
  await requireExpectedReceipt(options, expected);
}

export async function recoverInstallTransaction(options) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  const { journal } = tx;
  if (journal.operation !== "install") {
    throw transactionError("pending lifecycle operation does not match install");
  }
  await drainExistingTombstone(options, tx, dependencies, paths);
  await cleanupPartialStage(options, tx, dependencies, paths);
  const targetPresent = await pathExists(options.target);
  const stagePresent = await pathExists(paths.stage);
  if (targetPresent && stagePresent) {
    throw transactionError("install transaction state is ambiguous");
  }
  const receipt = await readOptionalReceipt(options);
  if (receipt !== undefined && !sameReceipt(receipt, journal.new_receipt)) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  if (!targetPresent && !stagePresent) {
    if (receipt !== undefined) {
      throw transactionError("install transaction state is ambiguous");
    }
    await tx.finish("install_complete");
    return { outcome: "rolled_back", journal: tx.journal };
  }
  if (stagePresent) {
    await options.verifyNew(paths.stage, journal);
    await tx.phase("install_before_promotion");
    await options.verifyNew(paths.stage, journal);
    await moveDurably({
      source: paths.stage,
      target: options.target,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "install promotion failed",
    });
    await tx.phase("install_target_promoted");
  }
  await options.verifyNew(options.target, journal);
  await options.register(options.target);
  if (receipt === undefined) {
    await tx.phase("install_before_receipt_commit");
    await persistExpectedReceipt(options, journal.new_receipt);
    await tx.phase("install_receipt_committed");
  } else {
    await requireExpectedReceipt(options, journal.new_receipt);
  }
  await tx.finish("install_complete");
  return { outcome: "committed", journal: tx.journal };
}

export async function rollbackInstallTransaction(options) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  await drainExistingTombstone(options, tx, dependencies, paths);
  await cleanupPartialStage(options, tx, dependencies, paths);
  const receipt = await readOptionalReceipt(options);
  if (receipt !== undefined) {
    if (!sameReceipt(receipt, options.journal.new_receipt)) {
      throw transactionError("lifecycle receipt does not match journal");
    }
    return recoverInstallTransaction({ ...options, journal: tx.journal });
  }
  const targetPresent = await pathExists(options.target);
  const stagePresent = await pathExists(paths.stage);
  if (targetPresent && stagePresent) {
    throw transactionError("install transaction state is ambiguous");
  }
  if (targetPresent) {
    await options.verifyNew(options.target, options.journal);
    await options.unregister(options.target);
    await quarantineAndGc({
      source: options.target,
      tombstone: paths.tombstones.target,
      verify: options.verifyNew,
      beforePhase: "install_before_target_cleanup",
      tombstonedPhase: "install_target_tombstoned",
      options,
      tx,
      dependencies,
    });
  } else if (stagePresent) {
    await quarantineAndGc({
      source: paths.stage,
      tombstone: paths.tombstones.stage,
      verify: options.verifyNew,
      beforePhase: "install_before_stage_cleanup",
      tombstonedPhase: "install_stage_tombstoned",
      options,
      tx,
      dependencies,
    });
  }
  await tx.finish("install_complete");
  return { outcome: "rolled_back", journal: tx.journal };
}

async function classifyReplacementState(options, paths) {
  const [targetPresent, stagePresent, backupPresent] = await Promise.all([
    pathExists(options.target),
    pathExists(paths.stage),
    pathExists(paths.backup),
  ]);
  const targetKind = targetPresent
    ? await options.classifyTarget(options.target, options.journal, {
        backupPresent,
        stagePresent,
      })
    : undefined;
  if (stagePresent) await options.verifyNew(paths.stage, options.journal);
  if (backupPresent) await options.verifyOld(paths.backup, options.journal);
  return { targetPresent, stagePresent, backupPresent, targetKind };
}

async function finishReplacementCommit(
  options,
  tx,
  state,
  dependencies,
  paths,
) {
  const prefix = options.journal.operation;
  if (state.targetKind === "old") {
    if (state.backupPresent || !state.stagePresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
    await options.verifyOld(options.target, options.journal);
    await tx.phase(`${prefix}_before_backup`);
    await options.verifyOld(options.target, options.journal);
    await moveDurably({
      source: options.target,
      target: paths.backup,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "installed App backup failed",
    });
    state.targetPresent = false;
    state.backupPresent = true;
    state.targetKind = undefined;
    await tx.phase(`${prefix}_backup_ready`);
  }
  if (!state.targetPresent) {
    if (!state.backupPresent || !state.stagePresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
    await options.verifyNew(paths.stage, options.journal);
    await tx.phase(`${prefix}_before_promotion`);
    await options.verifyNew(paths.stage, options.journal);
    await moveDurably({
      source: paths.stage,
      target: options.target,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "replacement promotion failed",
    });
    state.targetPresent = true;
    state.stagePresent = false;
    state.targetKind = "new";
    await tx.phase(`${prefix}_target_promoted`);
  }
  if (state.targetKind !== "new" || state.stagePresent) {
    throw transactionError("replacement transaction state is ambiguous");
  }
  await options.verifyNew(options.target, options.journal);
  await options.register(options.target);
  const receipt = await readOptionalReceipt(options);
  const receiptIsBoth = sameReceipt(
    options.journal.old_receipt,
    options.journal.new_receipt,
  );
  if (!receiptIsBoth && sameReceipt(receipt, options.journal.old_receipt)) {
    await tx.phase(`${prefix}_before_receipt_commit`);
    await persistExpectedReceipt(options, options.journal.new_receipt);
    await tx.phase(`${prefix}_receipt_committed`);
  } else if (!sameReceipt(receipt, options.journal.new_receipt)) {
    throw transactionError("lifecycle receipt does not match journal");
  } else if (
    new Set([
      `${prefix}_prepared`,
      `${prefix}_before_stage_publish`,
      `${prefix}_stage_ready`,
      `${prefix}_before_backup`,
      `${prefix}_backup_ready`,
      `${prefix}_before_promotion`,
      `${prefix}_target_promoted`,
      `${prefix}_before_receipt_commit`,
    ]).has(tx.journal.phase)
  ) {
    await tx.phase(`${prefix}_receipt_committed`);
  }
  if (state.backupPresent) {
    await quarantineAndGc({
      source: paths.backup,
      tombstone: paths.tombstones.backup,
      verify: options.verifyOld,
      beforePhase: `${prefix}_before_backup_cleanup`,
      tombstonedPhase: `${prefix}_backup_tombstoned`,
      options,
      tx,
      dependencies,
    });
  }
  await tx.finish(`${prefix}_complete`);
  return { outcome: "committed", journal: tx.journal };
}

async function recoverReplacementTransaction(options, operation) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  if (options.journal.operation !== operation) {
    throw transactionError(
      `pending lifecycle operation does not match ${operation}`,
    );
  }
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  await drainExistingTombstone(options, tx, dependencies, paths);
  await cleanupPartialStage(options, tx, dependencies, paths);
  const receipt = await readOptionalReceipt(options);
  if (
    !sameReceipt(receipt, options.journal.old_receipt) &&
    !sameReceipt(receipt, options.journal.new_receipt)
  ) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  const state = await classifyReplacementState(options, paths);
  if (state.targetKind === "old") {
    if (state.backupPresent || !sameReceipt(receipt, options.journal.old_receipt)) {
      throw transactionError("replacement transaction state is ambiguous");
    }
    if (!state.stagePresent) {
      await tx.finish(`${operation}_complete`);
      return { outcome: "rolled_back", journal: tx.journal };
    }
  } else if (!state.targetPresent) {
    if (!state.backupPresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
    if (!state.stagePresent) {
      if (!sameReceipt(receipt, options.journal.old_receipt)) {
        throw transactionError("replacement transaction state is ambiguous");
      }
      await tx.phase(`${operation}_before_restore`);
      await options.verifyOld(paths.backup, options.journal);
      await moveDurably({
        source: paths.backup,
        target: options.target,
        move: dependencies.move,
        directory: options.applicationsRoot,
        syncDirectory: dependencies.syncDirectory,
        message: "old App restoration failed",
      });
      await options.register(options.target);
      await tx.finish(`${operation}_complete`);
      return { outcome: "rolled_back", journal: tx.journal };
    }
  } else if (state.targetKind === "new") {
    if (state.stagePresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
  } else {
    throw transactionError("replacement transaction state is ambiguous");
  }
  return finishReplacementCommit(options, tx, state, dependencies, paths);
}

export function recoverReinstallTransaction(options) {
  return recoverReplacementTransaction(options, "reinstall");
}

async function rollbackReplacementTransaction(options, operation) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  await drainExistingTombstone(options, tx, dependencies, paths);
  await cleanupPartialStage(options, tx, dependencies, paths);
  const receipt = await readOptionalReceipt(options);
  if (sameReceipt(receipt, options.journal.new_receipt)) {
    return recoverReplacementTransaction(
      { ...options, journal: tx.journal },
      operation,
    );
  }
  if (!sameReceipt(receipt, options.journal.old_receipt)) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  const state = await classifyReplacementState(options, paths);
  if (state.targetKind === "new" && !state.backupPresent) {
    return recoverReplacementTransaction(
      { ...options, journal: tx.journal },
      operation,
    );
  }
  if (state.targetKind === "old") {
    if (state.backupPresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
  } else if (state.targetKind === "new") {
    if (!state.backupPresent || state.stagePresent) {
      throw transactionError("replacement transaction state is ambiguous");
    }
    await options.verifyNew(options.target, options.journal);
    await options.unregister(options.target);
    await quarantineAndGc({
      source: options.target,
      tombstone: paths.tombstones.target,
      verify: options.verifyNew,
      beforePhase: `${operation}_before_recovery_target_cleanup`,
      tombstonedPhase: `${operation}_target_tombstoned`,
      options,
      tx,
      dependencies,
    });
    state.targetPresent = false;
    state.targetKind = undefined;
  } else if (!state.backupPresent) {
    throw transactionError("replacement transaction state is ambiguous");
  }
  if (state.stagePresent) {
    await quarantineAndGc({
      source: paths.stage,
      tombstone: paths.tombstones.stage,
      verify: options.verifyNew,
      beforePhase: `${operation}_before_stage_cleanup`,
      tombstonedPhase: `${operation}_stage_tombstoned`,
      options,
      tx,
      dependencies,
    });
    state.stagePresent = false;
  }
  if (!state.targetPresent && state.backupPresent) {
    await options.verifyOld(paths.backup, options.journal);
    await tx.phase(`${operation}_before_restore`);
    await options.verifyOld(paths.backup, options.journal);
    await moveDurably({
      source: paths.backup,
      target: options.target,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "old App restoration failed",
    });
    await options.register(options.target);
  }
  await tx.finish(`${operation}_complete`);
  return { outcome: "rolled_back", journal: tx.journal };
}

export function rollbackReinstallTransaction(options) {
  return rollbackReplacementTransaction(options, "reinstall");
}

export async function recoverUninstallTransaction(options) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  if (options.journal.operation !== "uninstall") {
    throw transactionError("pending lifecycle operation does not match uninstall");
  }
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  await drainExistingTombstone(options, tx, dependencies, paths);
  let targetPresent = await pathExists(options.target);
  let stagePresent = await pathExists(paths.stage);
  if (targetPresent && stagePresent) {
    throw transactionError("uninstall transaction state is ambiguous");
  }
  let receipt = await readOptionalReceipt(options);
  if (receipt !== undefined && !sameReceipt(receipt, options.journal.old_receipt)) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  if (targetPresent) {
    if (receipt === undefined) {
      throw transactionError("uninstall transaction state is ambiguous");
    }
    await options.verifyOld(options.target, options.journal);
    await tx.phase("uninstall_before_quarantine");
    await options.verifyOld(options.target, options.journal);
    await options.unregister(options.target);
    await moveDurably({
      source: options.target,
      target: paths.stage,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "uninstall quarantine failed",
    });
    targetPresent = false;
    stagePresent = true;
    await tx.phase("uninstall_quarantined");
  }
  if (stagePresent) {
    await options.verifyOld(paths.stage, options.journal);
    if (receipt !== undefined) {
      await tx.phase("uninstall_before_receipt_removal");
      await options.removeReceipt({
        applicationSupportRoot: options.applicationSupportRoot,
        expectedReceipt: options.journal.old_receipt,
      });
      receipt = await readOptionalReceipt(options);
      if (receipt !== undefined) {
        throw transactionError("lifecycle receipt removal is incomplete");
      }
      await tx.phase("uninstall_receipt_removed");
    }
    await quarantineAndGc({
      source: paths.stage,
      tombstone: paths.tombstones.stage,
      verify: options.verifyOld,
      beforePhase: "uninstall_before_quarantine_cleanup",
      tombstonedPhase: "uninstall_quarantine_tombstoned",
      options,
      tx,
      dependencies,
    });
    await tx.finish("uninstall_complete");
    return { outcome: "committed", journal: tx.journal };
  }
  if (targetPresent || receipt !== undefined) {
    throw transactionError("uninstall transaction state is ambiguous");
  }
  await tx.finish("uninstall_complete");
  return { outcome: "committed", journal: tx.journal };
}

export async function rollbackUninstallTransaction(options) {
  requireLifecycleLockCapability(options.lifecycleLockCapability);
  const dependencies = transactionDependencies(options);
  const tx = controller({ ...options, ...dependencies });
  const paths = workspace(options);
  await drainExistingTombstone(options, tx, dependencies, paths);
  const [targetPresent, stagePresent] = await Promise.all([
    pathExists(options.target),
    pathExists(paths.stage),
  ]);
  if (targetPresent && stagePresent) {
    throw transactionError("uninstall transaction state is ambiguous");
  }
  let receipt = await readOptionalReceipt(options);
  if (receipt !== undefined && !sameReceipt(receipt, options.journal.old_receipt)) {
    throw transactionError("lifecycle receipt does not match journal");
  }
  if (!targetPresent && !stagePresent) {
    if (receipt === undefined) {
      await tx.finish("uninstall_complete");
      return { outcome: "committed", journal: tx.journal };
    }
    throw transactionError("uninstall transaction state is ambiguous");
  }
  const app = targetPresent ? options.target : paths.stage;
  await options.verifyOld(app, options.journal);
  if (receipt === undefined) {
    await tx.phase("uninstall_before_receipt_restore");
    await persistExpectedReceipt(options, options.journal.old_receipt);
    receipt = options.journal.old_receipt;
  }
  if (!targetPresent) {
    await tx.phase("uninstall_before_restore");
    await options.verifyOld(paths.stage, options.journal);
    await moveDurably({
      source: paths.stage,
      target: options.target,
      move: dependencies.move,
      directory: options.applicationsRoot,
      syncDirectory: dependencies.syncDirectory,
      message: "uninstall restoration failed",
    });
  }
  await options.register(options.target);
  await requireExpectedReceipt(options, options.journal.old_receipt);
  await tx.finish("uninstall_complete");
  return { outcome: "rolled_back", journal: tx.journal };
}
