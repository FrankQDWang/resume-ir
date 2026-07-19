import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  advanceLifecycleJournal,
  createLifecycleJournal,
  lifecycleJournalPath,
  persistLifecycleJournal,
  readLifecycleJournal,
  removeLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";

const TARGET = "aarch64-apple-darwin";
const OLD_DIGEST = "a".repeat(64);
const NEW_DIGEST = "b".repeat(64);

function receipt(version, compositionDigest, dmg = "c".repeat(64)) {
  return {
    schema_version: "resume-ir.macos-install-receipt.v1",
    bundle_id: "local.resume-ir.desktop",
    version,
    target_triple: TARGET,
    composition_digest: compositionDigest,
    dmg_sha256: dmg,
  };
}

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-journal-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationSupportRoot = path.join(root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  return { applicationSupportRoot, root };
}

function upgradeJournal() {
  return createLifecycleJournal({
    operation: "upgrade",
    phase: "upgrade_prepared",
    oldVersion: "0.1.1",
    newVersion: "0.1.2",
    oldCompositionDigest: OLD_DIGEST,
    newCompositionDigest: NEW_DIGEST,
    oldReceipt: receipt("0.1.1", OLD_DIGEST, "d".repeat(64)),
    newReceipt: receipt("0.1.2", NEW_DIGEST),
  });
}

test("persists one canonical owner-only journal and advances only its phase", async (context) => {
  const values = await fixture(context);
  const prepared = upgradeJournal();
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: prepared,
  });
  assert.deepEqual(
    await readLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    prepared,
  );
  assert.equal(
    (await stat(lifecycleJournalPath(values.applicationSupportRoot))).mode & 0o077,
    0,
  );
  assert.equal(
    await readFile(lifecycleJournalPath(values.applicationSupportRoot), "utf8"),
    `${JSON.stringify(prepared)}\n`,
  );

  const advanced = advanceLifecycleJournal({
    journal: prepared,
    phase: "upgrade_before_backup",
  });
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: advanced,
  });
  assert.deepEqual(
    await readLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    advanced,
  );
});

test("rejects unknown, non-canonical, corrupt, or transaction-drifted journals", async (context) => {
  const values = await fixture(context);
  assert.throws(
    () =>
      createLifecycleJournal({
        operation: "upgrade",
        phase: "upgrade_prepared",
        oldVersion: "0.1.2",
        newVersion: "0.1.1",
        oldCompositionDigest: OLD_DIGEST,
        newCompositionDigest: NEW_DIGEST,
        oldReceipt: receipt("0.1.2", OLD_DIGEST),
        newReceipt: receipt("0.1.1", NEW_DIGEST),
      }),
    /lifecycle journal is invalid/,
  );
  const journal = upgradeJournal();
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal,
  });
  const file = lifecycleJournalPath(values.applicationSupportRoot);
  await chmod(file, 0o644);
  await assert.rejects(
    readLifecycleJournal({ applicationSupportRoot: values.applicationSupportRoot }),
    /lifecycle journal is invalid/,
  );
  await chmod(file, 0o600);
  await writeFile(file, `${JSON.stringify(journal, null, 2)}\n`);
  await assert.rejects(
    readLifecycleJournal({ applicationSupportRoot: values.applicationSupportRoot }),
    /lifecycle journal is invalid/,
  );
  await writeFile(file, `${JSON.stringify(journal)}\n`);
  const evidenceDirectory = path.dirname(file);
  await chmod(evidenceDirectory, 0o777);
  await assert.rejects(
    readLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      allowMissing: true,
    }),
    /owner evidence directory is invalid/,
  );
  await chmod(evidenceDirectory, 0o700);
  await writeFile(file, `${JSON.stringify({ ...journal, unknown: true })}\n`);
  await assert.rejects(
    readLifecycleJournal({ applicationSupportRoot: values.applicationSupportRoot }),
    /lifecycle journal is invalid/,
  );

  await rm(file);
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal,
  });
  await assert.rejects(
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: { ...journal, new_version: "0.1.3" },
    }),
    /lifecycle journal transaction does not match/,
  );
});

test("restores journal bytes after post-rename or post-remove fsync failure", async (context) => {
  const values = await fixture(context);
  const prepared = upgradeJournal();
  await persistLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
    journal: prepared,
  });
  const advanced = advanceLifecycleJournal({
    journal: prepared,
    phase: "upgrade_before_backup",
  });
  let persistSyncs = 0;
  await assert.rejects(
    persistLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      journal: advanced,
      operations: {
        syncDirectory: async () => {
          persistSyncs += 1;
          if (persistSyncs === 1) throw new Error("synthetic fsync failure");
        },
      },
    }),
    /lifecycle journal could not be persisted/,
  );
  assert.deepEqual(
    await readLifecycleJournal({ applicationSupportRoot: values.applicationSupportRoot }),
    prepared,
  );

  let removeSyncs = 0;
  await assert.rejects(
    removeLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      expectedJournal: prepared,
      operations: {
        syncDirectory: async () => {
          removeSyncs += 1;
          if (removeSyncs === 1) throw new Error("synthetic fsync failure");
        },
      },
    }),
    /lifecycle journal could not be removed/,
  );
  assert.deepEqual(
    await readLifecycleJournal({ applicationSupportRoot: values.applicationSupportRoot }),
    prepared,
  );
});

test("creates exactly one transaction under a concurrent journal race", async (context) => {
  const values = await fixture(context);
  const first = upgradeJournal();
  const second = createLifecycleJournal({
    operation: "upgrade",
    phase: "upgrade_prepared",
    oldVersion: "0.1.1",
    newVersion: "0.1.2",
    oldCompositionDigest: OLD_DIGEST,
    newCompositionDigest: NEW_DIGEST,
    oldReceipt: receipt("0.1.1", OLD_DIGEST, "d".repeat(64)),
    newReceipt: receipt("0.1.2", NEW_DIGEST),
  });
  const outcomes = await Promise.allSettled(
    [first, second].map((journal) =>
      persistLifecycleJournal({
        applicationSupportRoot: values.applicationSupportRoot,
        journal,
      }),
    ),
  );
  assert.equal(outcomes.filter(({ status }) => status === "fulfilled").length, 1);
  assert.equal(outcomes.filter(({ status }) => status === "rejected").length, 1);
  assert.match(
    outcomes.find(({ status }) => status === "rejected").reason.message,
    /lifecycle journal already exists/,
  );
  const stored = await readLifecycleJournal({
    applicationSupportRoot: values.applicationSupportRoot,
  });
  assert.ok([first.transaction_id, second.transaction_id].includes(stored.transaction_id));
});
