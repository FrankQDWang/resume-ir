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
  recoverReinstallTransaction,
  recoverUninstallTransaction,
  rollbackInstallTransaction,
  rollbackReinstallTransaction,
  rollbackUninstallTransaction,
} from "./macos-lifecycle-transaction.mjs";
import {
  createInstallReceiptEvidence,
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

const VERSION = "0.1.2";
const OLD_SOURCE = Object.freeze({
  authority: "exact_main_commit",
  base_commit: "1".repeat(40),
  source_tree_sha256: "2".repeat(64),
});
const NEW_SOURCE = Object.freeze({
  authority: "worktree_snapshot",
  base_commit: "3".repeat(40),
  source_tree_sha256: "4".repeat(64),
});

function receipt(generation) {
  return {
    schema_version: "resume-ir.macos-install-receipt.v3",
    bundle_id: "local.resume-ir.desktop",
    version: VERSION,
    target_triple: "aarch64-apple-darwin",
    source: generation === "old" ? OLD_SOURCE : NEW_SOURCE,
    composition_digest: generation === "old" ? "5".repeat(64) : "6".repeat(64),
    dmg_sha256: generation === "old" ? "7".repeat(64) : "8".repeat(64),
  };
}

function journal(operation, phase) {
  if (operation === "install") {
    return createLifecycleJournal({
      operation,
      phase,
      newVersion: VERSION,
      newCompositionDigest: receipt("new").composition_digest,
      newReceipt: receipt("new"),
    });
  }
  if (operation === "uninstall") {
    return createLifecycleJournal({
      operation,
      phase,
      oldVersion: VERSION,
      oldCompositionDigest: receipt("old").composition_digest,
      oldReceipt: receipt("old"),
    });
  }
  return createLifecycleJournal({
    operation: "reinstall",
    phase,
    oldVersion: VERSION,
    newVersion: VERSION,
    oldCompositionDigest: receipt("old").composition_digest,
    newCompositionDigest: receipt("new").composition_digest,
    oldReceipt: receipt("old"),
    newReceipt: receipt("new"),
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
    applicationSupportRoot,
    applicationsRoot,
    lifecycleLockCapability,
    target: path.join(applicationsRoot, "resume-ir.app"),
  };
}

async function writeApp(target, generation) {
  await mkdir(target);
  await writeFile(path.join(target, "generation"), generation);
}

async function verifyGeneration(target, expected) {
  assert.equal(
    await readFile(path.join(target, "generation"), "utf8"),
    expected,
  );
}

function callbacks(values) {
  return {
    applicationSupportRoot: values.applicationSupportRoot,
    applicationsRoot: values.applicationsRoot,
    lifecycleLockCapability: values.lifecycleLockCapability,
    target: values.target,
    readReceipt: readInstallReceipt,
    persistJournal: persistLifecycleJournal,
    verifyOld: (target) => verifyGeneration(target, "old"),
    verifyNew: (target) => verifyGeneration(target, "new"),
    classifyTarget: async (target) => {
      const generation = await readFile(
        path.join(target, "generation"),
        "utf8",
      );
      if (!["old", "new"].includes(generation)) {
        throw new Error("target generation is invalid");
      }
      return generation;
    },
    register: async () => {},
    unregister: async () => {},
  };
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

test("every recovery mutation rejects a missing lifecycle lock capability", async () => {
  for (const operation of [
    recoverInstallTransaction,
    rollbackInstallTransaction,
    recoverReinstallTransaction,
    rollbackReinstallTransaction,
    recoverUninstallTransaction,
    rollbackUninstallTransaction,
  ]) {
    await assert.rejects(operation({}), /lifecycle lock capability is invalid/);
  }
});

test("reinstall resumes after the old target became the durable backup", async (context) => {
  const values = await fixture(context);
  const current = journal("reinstall", "reinstall_backup_ready");
  const paths = lifecycleWorkspacePaths({
    applicationsRoot: values.applicationsRoot,
    operation: "reinstall",
    transactionId: current.transaction_id,
  });
  await Promise.all([
    writeApp(paths.backup, "old"),
    writeApp(paths.ready, "new"),
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: current,
    }),
    createInstallReceiptEvidence({
      applicationSupportRoot: values.applicationSupportRoot,
      receipt: receipt("old"),
    }),
  ]);
  const options = {
    ...callbacks(values),
    journal: current,
    persistReceipt: ({ applicationSupportRoot, receipt: next }) =>
      persistInstallReceipt({
        applicationSupportRoot,
        receipt: next,
        expectedReceipt: receipt("old"),
      }),
  };

  const result = await recoverReinstallTransaction(options);
  assert.equal(result.outcome, "committed");
  await verifyGeneration(values.target, "new");
  assert.deepEqual(
    await readInstallReceipt({
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    receipt("new"),
  );
  assert.deepEqual(await readdir(values.applicationsRoot), ["resume-ir.app"]);
  await assertJournalRemoved(values);
});

test("install resumes from a durable stage and commits its v3 receipt", async (context) => {
  const values = await fixture(context);
  const current = journal("install", "install_stage_ready");
  const paths = lifecycleWorkspacePaths({
    applicationsRoot: values.applicationsRoot,
    operation: "install",
    transactionId: current.transaction_id,
  });
  await Promise.all([
    writeApp(paths.ready, "new"),
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: current,
    }),
  ]);

  const result = await recoverInstallTransaction({
    ...callbacks(values),
    journal: current,
    persistReceipt: createInstallReceiptEvidence,
  });
  assert.equal(result.outcome, "committed");
  await verifyGeneration(values.target, "new");
  assert.deepEqual(
    await readInstallReceipt({
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    receipt("new"),
  );
  await assertJournalRemoved(values);
});

test("uninstall resumes from quarantine without deleting user evidence", async (context) => {
  const values = await fixture(context);
  const current = journal("uninstall", "uninstall_quarantined");
  const paths = lifecycleWorkspacePaths({
    applicationsRoot: values.applicationsRoot,
    operation: "uninstall",
    transactionId: current.transaction_id,
  });
  await Promise.all([
    writeApp(paths.quarantine, "old"),
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: current,
    }),
    createInstallReceiptEvidence({
      applicationSupportRoot: values.applicationSupportRoot,
      receipt: receipt("old"),
    }),
  ]);

  const result = await recoverUninstallTransaction({
    ...callbacks(values),
    journal: current,
    removeReceipt: removeInstallReceipt,
    persistReceipt: createInstallReceiptEvidence,
  });
  assert.equal(result.outcome, "committed");
  assert.deepEqual(await readdir(values.applicationsRoot), []);
  assert.equal(
    await readInstallReceipt({
      applicationSupportRoot: values.applicationSupportRoot,
      allowMissing: true,
    }),
    undefined,
  );
  await assertJournalRemoved(values);
});

test("a mismatched v3 receipt fails closed before replacing either generation", async (context) => {
  const values = await fixture(context);
  const current = journal("reinstall", "reinstall_backup_ready");
  const paths = lifecycleWorkspacePaths({
    applicationsRoot: values.applicationsRoot,
    operation: "reinstall",
    transactionId: current.transaction_id,
  });
  await Promise.all([
    writeApp(paths.backup, "old"),
    writeApp(paths.ready, "new"),
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: current,
    }),
    createInstallReceiptEvidence({
      applicationSupportRoot: values.applicationSupportRoot,
      receipt: { ...receipt("old"), dmg_sha256: "9".repeat(64) },
    }),
  ]);

  await assert.rejects(
    recoverReinstallTransaction({
      ...callbacks(values),
      journal: current,
      persistReceipt: () => {
        throw new Error("must not persist");
      },
    }),
    /receipt does not match journal/,
  );
  await verifyGeneration(paths.backup, "old");
  await verifyGeneration(paths.ready, "new");
});
