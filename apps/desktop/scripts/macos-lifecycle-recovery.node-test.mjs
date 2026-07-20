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
  createInstallReceiptEvidence,
  persistInstallReceipt,
  readInstallReceipt,
  removeInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  LEGACY_EXACT_COMPOSITION_DIGEST,
  LEGACY_EXACT_DMG_SHA256,
  LEGACY_INSTALL_RECEIPT_FILE,
  legacyExactInstallReceiptPath,
  readInstallReceiptSet,
  readLegacyExactInstallReceipt,
  removeLegacyExactInstallReceipt,
  validateLegacyExactInstallReceipt,
} from "./macos-legacy-exact-artifact.mjs";
import {
  acquireLifecycleLock,
  prepareLifecycleLockFile,
  releaseLifecycleLock,
} from "./macos-lifecycle-lock.mjs";
import {
  persistOwnerEvidence,
  prepareOwnerEvidenceDirectory,
} from "./macos-owner-evidence-store.mjs";
import { lifecycleWorkspacePaths } from "./macos-lifecycle-workspace.mjs";

const OLD_VERSION = "0.1.1";
const NEW_VERSION = "0.1.2";
const OLD_DIGEST = LEGACY_EXACT_COMPOSITION_DIGEST;
const NEW_DIGEST = "b".repeat(64);
const SOURCE_COMMIT = "e".repeat(40);

function receipt(version) {
  if (version === OLD_VERSION) {
    return {
      schema_version: "resume-ir.macos-install-receipt.v1",
      bundle_id: "local.resume-ir.desktop",
      version,
      target_triple: "aarch64-apple-darwin",
      composition_digest: OLD_DIGEST,
      dmg_sha256: LEGACY_EXACT_DMG_SHA256,
    };
  }
  return {
    schema_version: "resume-ir.macos-install-receipt.v2",
    bundle_id: "local.resume-ir.desktop",
    version,
    target_triple: "aarch64-apple-darwin",
    source_commit: SOURCE_COMMIT,
    composition_digest: NEW_DIGEST,
    dmg_sha256: "d".repeat(64),
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
      oldVersion: NEW_VERSION,
      oldCompositionDigest: NEW_DIGEST,
      oldReceipt: receipt(NEW_VERSION),
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

async function persistLegacyReceipt({ applicationSupportRoot, receipt: value }) {
  return persistOwnerEvidence({
    applicationSupportRoot,
    fileName: LEGACY_INSTALL_RECEIPT_FILE,
    value,
    maxBytes: 4 * 1024,
    validate: validateLegacyExactInstallReceipt,
    label: "legacy exact install receipt",
  });
}

function callbacks(values) {
  const calls = { old: [], current: [], register: [], unregister: [] };
  const verifyOld = async (target) => {
    await requireVersion(target, NEW_VERSION);
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
      if (version === NEW_VERSION) {
        await verifyNew(target);
        return "new";
      }
      throw new Error("target version is invalid");
    },
    register: async (target) => calls.register.push(target),
    unregister: async (target) => calls.unregister.push(target),
    readReceipt: readInstallReceipt,
    persistReceipt: createInstallReceiptEvidence,
    removeReceipt: removeInstallReceipt,
    applicationSupportRoot: values.applicationSupportRoot,
    applicationsRoot: values.applicationsRoot,
    lifecycleLockCapability: values.lifecycleLockCapability,
  };
}

function upgradeCallbacks(values) {
  const base = callbacks(values);
  const verifyOld = async (target) => {
    await requireVersion(target, OLD_VERSION);
    base.calls.old.push(target);
  };
  const verifyNew = async (target) => {
    await requireVersion(target, NEW_VERSION);
    base.calls.current.push(target);
  };
  const inspectReceiptSet = () =>
    readInstallReceiptSet({
      applicationSupportRoot: values.applicationSupportRoot,
    });
  return {
    ...base,
    verifyOld,
    verifyNew,
    classifyTarget: async (target) => {
      const [version, receiptSet] = await Promise.all([
        readFile(path.join(target, "version"), "utf8"),
        inspectReceiptSet(),
      ]);
      if (version === OLD_VERSION) {
        if (receiptSet.state !== "legacy_only") {
          throw new Error("upgrade transaction state is ambiguous");
        }
        await verifyOld(target);
        return "old";
      }
      if (version === NEW_VERSION) {
        await verifyNew(target);
        return "new";
      }
      throw new Error("target version is invalid");
    },
    readReceipt: async () => {
      const receiptSet = await inspectReceiptSet();
      return receiptSet.state === "legacy_only"
        ? receiptSet.legacy_receipt
        : receiptSet.current_receipt;
    },
    persistReceipt: createInstallReceiptEvidence,
    readReceiptSet: inspectReceiptSet,
    removeLegacyReceipt: removeLegacyExactInstallReceipt,
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
  await persistLegacyReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: prepared,
  });
  const base = {
    ...upgradeCallbacks(values),
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
  assert.equal(
    (await readInstallReceiptSet({
      applicationSupportRoot: values.applicationSupportRoot,
    })).state,
    "current_only",
  );
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("repeated upgrade crashes preserve receipt commit ordering and resume safely", async (context) => {
  const values = await fixture(context);
  let current = journal("upgrade", "upgrade_stage_ready");
  const paths = pathsFor(values, current);
  await writeApp(values.target, OLD_VERSION);
  await writeApp(paths.ready, NEW_VERSION);
  await persistLegacyReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const base = {
    ...upgradeCallbacks(values),
    target: values.target,
  };
  const crashCases = [
    {
      phase: "upgrade_target_promoted",
      journalPhase: "upgrade_before_promotion",
      receiptState: "legacy_only",
    },
    {
      phase: "upgrade_receipt_committed",
      journalPhase: "upgrade_before_receipt_commit",
      receiptState: "both_valid",
    },
    {
      phase: "upgrade_before_legacy_receipt_removal",
      journalPhase: "upgrade_receipt_committed",
      receiptState: "both_valid",
    },
    {
      phase: "upgrade_legacy_receipt_removed",
      journalPhase: "upgrade_before_legacy_receipt_removal",
      receiptState: "current_only",
    },
    {
      phase: "upgrade_complete",
      journalPhase: "upgrade_backup_tombstoned",
      receiptState: "current_only",
    },
  ];
  for (const crashCase of crashCases) {
    await assert.rejects(
      recoverUpgradeTransaction({
        ...base,
        journal: current,
        persistJournal: failOnceAtPhase(crashCase.phase),
      }),
      /simulated crash/,
    );
    current = await readJournal(values);
    assert.equal(current.phase, crashCase.journalPhase);
    assert.equal(
      (await readInstallReceiptSet({
        applicationSupportRoot: values.applicationSupportRoot,
      })).state,
      crashCase.receiptState,
    );
  }
  await recoverUpgradeTransaction({ ...base, journal: current });
  await requireVersion(values.target, NEW_VERSION);
  assert.deepEqual(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), receipt(NEW_VERSION));
  assert.equal(
    (await readInstallReceiptSet({
      applicationSupportRoot: values.applicationSupportRoot,
    })).state,
    "current_only",
  );
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
  await persistLegacyReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });
  const result = await rollbackUpgradeTransaction({
    ...upgradeCallbacks(values),
    journal: current,
    target: values.target,
  });
  assert.equal(result.outcome, "rolled_back");
  await requireVersion(values.target, OLD_VERSION);
  assert.equal(
    await readFile(
      legacyExactInstallReceiptPath(values.applicationSupportRoot),
      "utf8",
    ),
    `${JSON.stringify(receipt(OLD_VERSION))}\n`,
  );
  assert.deepEqual(await readLegacyExactInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), receipt(OLD_VERSION));
  assert.equal(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    allowMissing: true,
  }), undefined);
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
  const verification = upgradeCallbacks(values);
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
  await writeApp(values.target, NEW_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(NEW_VERSION),
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
    ...upgradeCallbacks(values),
    journal: current,
    target: values.target,
  };
  await assert.rejects(recoverUpgradeTransaction(base));
  await requireVersion(values.target, NEW_VERSION);
  await requireVersion(paths.backup, "9.9.9");
  assert.deepEqual(await readJournal(values), current);
});

test("old target with a v2 receipt fails closed and preserves both generations", async (context) => {
  const values = await fixture(context);
  const current = journal("upgrade", "upgrade_stage_ready");
  const paths = pathsFor(values, current);
  await writeApp(values.target, OLD_VERSION);
  await writeApp(paths.ready, NEW_VERSION);
  await persistLegacyReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(OLD_VERSION),
  });
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: receipt(NEW_VERSION),
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });

  await assert.rejects(
    recoverUpgradeTransaction({
      ...upgradeCallbacks(values),
      journal: current,
      target: values.target,
    }),
    /upgrade transaction state is ambiguous/,
  );
  await requireVersion(values.target, OLD_VERSION);
  await requireVersion(paths.ready, NEW_VERSION);
  assert.equal(
    (await readInstallReceiptSet({
      applicationSupportRoot: values.applicationSupportRoot,
    })).state,
    "both_valid",
  );
  assert.deepEqual(await readJournal(values), current);
});

test("new target without either receipt fails closed and preserves App evidence", async (context) => {
  const values = await fixture(context);
  const current = journal("upgrade", "upgrade_target_promoted");
  const paths = pathsFor(values, current);
  await writeApp(values.target, NEW_VERSION);
  await writeApp(paths.backup, OLD_VERSION);
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });

  await assert.rejects(
    recoverUpgradeTransaction({
      ...upgradeCallbacks(values),
      journal: current,
      target: values.target,
    }),
    /install receipt set is invalid/,
  );
  await requireVersion(values.target, NEW_VERSION);
  await requireVersion(paths.backup, OLD_VERSION);
  assert.deepEqual(await readJournal(values), current);
});

test("mismatched v2 receipt fails closed without changing App or receipt bytes", async (context) => {
  const values = await fixture(context);
  const current = journal("upgrade", "upgrade_receipt_committed");
  const paths = pathsFor(values, current);
  const mismatched = {
    ...receipt(NEW_VERSION),
    composition_digest: "f".repeat(64),
  };
  await writeApp(values.target, NEW_VERSION);
  await writeApp(paths.backup, OLD_VERSION);
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: mismatched,
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: current,
  });

  await assert.rejects(
    recoverUpgradeTransaction({
      ...upgradeCallbacks(values),
      journal: current,
      target: values.target,
    }),
    /lifecycle receipt does not match journal/,
  );
  await requireVersion(values.target, NEW_VERSION);
  await requireVersion(paths.backup, OLD_VERSION);
  assert.deepEqual(await readInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
  }), mismatched);
  assert.deepEqual(await readJournal(values), current);
});
