import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  createLifecycleJournal,
  persistLifecycleJournal,
  readLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";
import {
  recoverInstallTransaction,
  recoverUninstallTransaction,
  recoverUpgradeTransaction,
  rollbackInstallTransaction,
  rollbackUninstallTransaction,
  rollbackUpgradeTransaction,
} from "./macos-lifecycle-transaction.mjs";
import {
  persistInstallReceipt,
  readInstallReceipt,
  removeInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  acquireLifecycleLock,
  prepareLifecycleLockFile,
  releaseLifecycleLock,
} from "./macos-lifecycle-lock.mjs";
import { prepareOwnerEvidenceDirectory } from "./macos-owner-evidence-store.mjs";
import { lifecycleWorkspacePaths } from "./macos-lifecycle-workspace.mjs";

const OLD_VERSION = "0.1.1";
const NEW_VERSION = "0.1.2";
const OLD_DIGEST = "a".repeat(64);
const NEW_DIGEST = "b".repeat(64);

function receipt(version) {
  return {
    schema_version: "resume-ir.macos-install-receipt.v1",
    bundle_id: "local.resume-ir.desktop",
    version,
    target_triple: "aarch64-apple-darwin",
    composition_digest: version === OLD_VERSION ? OLD_DIGEST : NEW_DIGEST,
    dmg_sha256: version === OLD_VERSION ? "c".repeat(64) : "d".repeat(64),
  };
}

function journal(operation, phase) {
  if (operation === "install") {
    return createLifecycleJournal({
      operation,
      phase,
      newVersion: NEW_VERSION,
      newCompositionDigest: NEW_DIGEST,
      newReceipt: receipt(NEW_VERSION),
    });
  }
  if (operation === "uninstall") {
    return createLifecycleJournal({
      operation,
      phase,
      oldVersion: OLD_VERSION,
      oldCompositionDigest: OLD_DIGEST,
      oldReceipt: receipt(OLD_VERSION),
    });
  }
  return createLifecycleJournal({
    operation,
    phase,
    oldVersion: OLD_VERSION,
    newVersion: NEW_VERSION,
    oldCompositionDigest: OLD_DIGEST,
    newCompositionDigest: NEW_DIGEST,
    oldReceipt: receipt(OLD_VERSION),
    newReceipt: receipt(NEW_VERSION),
  });
}

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-recovery-test-")),
  );
  const applicationsRoot = path.join(root, "Applications");
  const applicationSupportRoot = path.join(
    root,
    "Library",
    "Application Support",
  );
  await mkdir(applicationsRoot);
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const lockFile = await prepareLifecycleLockFile({
    applicationSupportRoot,
    prepareEvidenceDirectory: prepareOwnerEvidenceDirectory,
  });
  const lifecycleLockCapability = await acquireLifecycleLock({ lockFile });
  context.after(async () => {
    await releaseLifecycleLock(lifecycleLockCapability);
    await rm(root, { recursive: true, force: true });
  });
  return {
    applicationsRoot,
    applicationSupportRoot,
    lifecycleLockCapability,
    target: path.join(applicationsRoot, "resume-ir.app"),
  };
}

async function writeApp(target, version) {
  await mkdir(target);
  await writeFile(path.join(target, "version"), version);
}

async function requireVersion(target, expected) {
  assert.equal(await readFile(path.join(target, "version"), "utf8"), expected);
}

function callbacks(values) {
  const calls = { old: [], current: [], register: [], unregister: [] };
  const verifyOld = async (target) => {
    await requireVersion(target, OLD_VERSION);
    calls.old.push(target);
  };
  const verifyNew = async (target) => {
    await requireVersion(target, NEW_VERSION);
    calls.current.push(target);
  };
  return {
    calls,
    verifyOld,
    verifyNew,
    classifyTarget: async (target) => {
      const version = await readFile(path.join(target, "version"), "utf8");
      if (version === OLD_VERSION) {
        await verifyOld(target);
        return "old";
      }
      if (version === NEW_VERSION) {
        await verifyNew(target);
        return "new";
      }
      throw new Error("target version is invalid");
    },
    register: async (target) => calls.register.push(target),
    unregister: async (target) => calls.unregister.push(target),
    readReceipt: readInstallReceipt,
    persistReceipt: persistInstallReceipt,
    removeReceipt: removeInstallReceipt,
    applicationSupportRoot: values.applicationSupportRoot,
    applicationsRoot: values.applicationsRoot,
    lifecycleLockCapability: values.lifecycleLockCapability,
  };
}

function failOnceAtPhase(phase) {
  let failed = false;
  return async (request) => {
    if (!failed && request.journal.phase === phase) {
      failed = true;
      throw new Error(`simulated crash after ${phase}`);
    }
    return persistLifecycleJournal(request);
  };
}

async function readJournal(values) {
  return readLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
  });
}

async function assertJournalRemoved(values) {
  assert.equal(
    await readLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      allowMissing: true,
    }),
    undefined,
  );
}

function pathsFor(values, current) {
  return lifecycleWorkspacePaths({
    applicationsRoot: values.applicationsRoot,
    operation: current.operation,
    transactionId: current.transaction_id,
  });
}

test("every recovery mutation rejects a missing lifecycle lock capability", async () => {
  for (const operation of [
    recoverInstallTransaction,
    rollbackInstallTransaction,
    recoverUpgradeTransaction,
    rollbackUpgradeTransaction,
    recoverUninstallTransaction,
    rollbackUninstallTransaction,
  ]) {
    await assert.rejects(
      operation({}),
      /lifecycle lock capability is invalid/,
    );
  }
});

test("resumes upgrade after target-to-backup crash", async (context) => {
  const values = await fixture(context);
  const prepared = journal("upgrade", "upgrade_stage_ready");
  const paths = pathsFor(values, prepared);
  await writeApp(values.target, OLD_VERSION);
  await writeApp(paths.ready, NEW_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: prepared,
  });
  const base = {
    ...callbacks(values),
    journal: prepared,
    target: values.target,
  };
  await assert.rejects(
    recoverUpgradeTransaction({
      ...base,
      persistJournal: failOnceAtPhase("upgrade_backup_ready"),
    }),
    /simulated crash/,
  );
  assert.deepEqual((await readdir(values.applicationsRoot)).sort(), [
    path.basename(paths.backup),
    path.basename(paths.ready),
  ]);

  await recoverUpgradeTransaction({ ...base, journal: await readJournal(values) });
  await requireVersion(values.target, NEW_VERSION);
  assert.deepEqual(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), receipt(NEW_VERSION));
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("repeated upgrade crashes resume after promotion, receipt commit, and backup cleanup", async (context) => {
  const values = await fixture(context);
  let current = journal("upgrade", "upgrade_stage_ready");
  const paths = pathsFor(values, current);
  await writeApp(values.target, OLD_VERSION);
  await writeApp(paths.ready, NEW_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const base = {
    ...callbacks(values),
    target: values.target,
  };
  for (const phase of [
    "upgrade_target_promoted",
    "upgrade_receipt_committed",
    "upgrade_complete",
  ]) {
    await assert.rejects(
      recoverUpgradeTransaction({
        ...base,
        journal: current,
        persistJournal: failOnceAtPhase(phase),
      }),
      /simulated crash/,
    );
    current = await readJournal(values);
  }
  await recoverUpgradeTransaction({ ...base, journal: current });
  await requireVersion(values.target, NEW_VERSION);
  assert.deepEqual(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), receipt(NEW_VERSION));
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("install resumes after receipt commit without deleting the promoted App", async (context) => {
  const values = await fixture(context);
  await writeApp(values.target, NEW_VERSION);
  let current = journal("install", "install_target_promoted");
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const base = {
    ...callbacks(values),
    journal: current,
    target: values.target,
  };
  await assert.rejects(
    recoverInstallTransaction({
      ...base,
      persistJournal: failOnceAtPhase("install_receipt_committed"),
    }),
    /simulated crash/,
  );
  current = await readJournal(values);
  assert.deepEqual(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), receipt(NEW_VERSION));
  await recoverInstallTransaction({ ...base, journal: current });
  await requireVersion(values.target, NEW_VERSION);
  await assertJournalRemoved(values);
});

test("invalid transaction-owned partial stage is quarantined and rolled back", async (context) => {
  const values = await fixture(context);
  const current = journal("upgrade", "upgrade_prepared");
  const paths = pathsFor(values, current);
  await writeApp(values.target, OLD_VERSION);
  await mkdir(paths.partial);
  await writeFile(path.join(paths.partial, "incomplete"), "partial-copy");
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  await recoverUpgradeTransaction({
    ...callbacks(values),
    journal: current,
    target: values.target,
  });
  await requireVersion(values.target, OLD_VERSION);
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("partial tombstone GC resumes without re-verifying deleted backup bytes", async (context) => {
  const values = await fixture(context);
  let current = journal("upgrade", "upgrade_receipt_committed");
  const paths = pathsFor(values, current);
  await writeApp(values.target, NEW_VERSION);
  await writeApp(paths.backup, OLD_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(NEW_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const verification = callbacks(values);
  let failed = false;
  await assert.rejects(
    recoverUpgradeTransaction({
      ...verification,
      journal: current,
      target: values.target,
      filesystem: {
        rm: async (target, options) => {
          if (!failed && target === paths.backupTombstone) {
            failed = true;
            await rm(path.join(target, "version"));
            throw new Error("simulated crash during tombstone GC");
          }
          await rm(target, options);
        },
      },
    }),
    /simulated crash/,
  );
  const oldVerificationCount = verification.calls.old.length;
  current = await readJournal(values);
  assert.equal(current.phase, "upgrade_backup_tombstoned");
  await recoverUpgradeTransaction({
    ...verification,
    journal: current,
    target: values.target,
  });
  assert.equal(verification.calls.old.length, oldVerificationCount);
  await requireVersion(values.target, NEW_VERSION);
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("uninstall resumes after quarantine and receipt removal crashes", async (context) => {
  const values = await fixture(context);
  await writeApp(values.target, OLD_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  let current = journal("uninstall", "uninstall_prepared");
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const base = {
    ...callbacks(values),
    target: values.target,
  };
  for (const phase of ["uninstall_quarantined", "uninstall_receipt_removed"]) {
    await assert.rejects(
      recoverUninstallTransaction({
        ...base,
        journal: current,
        persistJournal: failOnceAtPhase(phase),
      }),
      /simulated crash/,
    );
    current = await readJournal(values);
  }
  await recoverUninstallTransaction({ ...base, journal: current });
  assert.deepEqual(await readdir(values.applicationsRoot), []);
  assert.equal(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    allowMissing: true,
  }), undefined);
  await assertJournalRemoved(values);
});

test("tampered backup fails closed without deleting either surviving version", async (context) => {
  const values = await fixture(context);
  const current = journal("upgrade", "upgrade_receipt_committed");
  const paths = pathsFor(values, current);
  await writeApp(values.target, NEW_VERSION);
  await writeApp(paths.backup, "9.9.9");
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(NEW_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const base = {
    ...callbacks(values),
    journal: current,
    target: values.target,
  };
  await assert.rejects(recoverUpgradeTransaction(base));
  await requireVersion(values.target, NEW_VERSION);
  await requireVersion(paths.backup, "9.9.9");
  assert.deepEqual(await readJournal(values), current);
});
